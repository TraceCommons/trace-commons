//! HuggingFace parquet dataset reader. Lists parquet shards for a dataset id,
//! downloads them through `hf-hub`'s cache, and streams rows as JSON values
//! keyed by column name.

use std::path::PathBuf;

use anyhow::Context;
use serde_json::Value;

/// A single row decoded from a parquet shard. Fields are name -> JSON value;
/// translators pull what they need from the column map.
#[derive(Debug, Clone, Default)]
pub struct Row {
    pub fields: std::collections::BTreeMap<String, Value>,
}

impl Row {
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.fields.get(key).and_then(|v| v.as_str())
    }

    pub fn get_array_strs(&self, key: &str) -> Vec<String> {
        self.fields
            .get(key)
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn get_value(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }
}

/// Iterator over rows from one parquet shard.
pub struct ShardRowIter {
    inner: Box<dyn Iterator<Item = anyhow::Result<Row>> + Send>,
}

impl ShardRowIter {
    // Safe empty-iter default for callers that need to short-circuit a shard
    // without bailing the run. Not yet wired from the main loop.
    #[allow(dead_code)]
    pub fn empty() -> Self {
        Self {
            inner: Box::new(std::iter::empty()),
        }
    }
}

impl Iterator for ShardRowIter {
    type Item = anyhow::Result<Row>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

/// Discover parquet shards for the given HF dataset id, downloading them into
/// the cache if missing. Returns the local cache paths.
pub async fn list_parquet_shards(
    dataset_id: &str,
    cache_dir: Option<&std::path::Path>,
) -> anyhow::Result<Vec<PathBuf>> {
    use hf_hub::api::tokio::ApiBuilder;
    use hf_hub::{Repo, RepoType};

    // `from_env()` honors `HF_HOME` and `HF_ENDPOINT`. The latter lets the
    // operator smoke harness redirect dataset discovery at a loopback mock
    // without rebuilding; production runs leave both unset and pick up the
    // standard huggingface.co endpoint plus `~/.cache/huggingface`.
    let mut builder = ApiBuilder::from_env();
    if let Some(dir) = cache_dir {
        builder = builder.with_cache_dir(dir.to_path_buf());
    }
    let api = builder.build().with_context(|| "build hf-hub api")?;
    let repo = api.repo(Repo::new(dataset_id.to_string(), RepoType::Dataset));

    let info = repo
        .info()
        .await
        .with_context(|| format!("fetch HF dataset info for {dataset_id}"))?;

    let mut shards = Vec::new();
    for sibling in info.siblings {
        let name = sibling.rfilename;
        if name.ends_with(".parquet") {
            let local = repo
                .get(&name)
                .await
                .with_context(|| format!("download parquet shard {name}"))?;
            shards.push(local);
        }
    }
    if shards.is_empty() {
        anyhow::bail!("dataset {dataset_id} exposes no .parquet shards");
    }
    Ok(shards)
}

/// Stream rows from a local parquet file. Columns become JSON values keyed by
/// column name. Nested lists/structs are mapped through `arrow`-to-`serde_json`
/// best-effort.
pub fn stream_parquet_rows(path: &std::path::Path) -> anyhow::Result<ShardRowIter> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let file = std::fs::File::open(path)
        .with_context(|| format!("open parquet shard {}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("read parquet metadata for {}", path.display()))?;
    let reader = builder
        .build()
        .with_context(|| format!("build parquet reader for {}", path.display()))?;

    let iter = reader.flat_map(|batch| {
        match batch {
            Ok(batch) => batch_to_rows(batch),
            Err(err) => vec![Err(anyhow::Error::from(err))],
        }
        .into_iter()
    });
    Ok(ShardRowIter {
        inner: Box::new(iter),
    })
}

fn batch_to_rows(batch: arrow_array::RecordBatch) -> Vec<anyhow::Result<Row>> {
    let schema = batch.schema();
    let mut rows = Vec::with_capacity(batch.num_rows());
    for row_idx in 0..batch.num_rows() {
        let mut fields = std::collections::BTreeMap::new();
        for (col_idx, field) in schema.fields().iter().enumerate() {
            let array = batch.column(col_idx);
            let value = array_value_to_json(array.as_ref(), row_idx);
            fields.insert(field.name().clone(), value);
        }
        rows.push(Ok(Row { fields }));
    }
    rows
}

fn array_value_to_json(array: &dyn arrow_array::Array, row: usize) -> Value {
    use arrow_array::cast::AsArray;
    use arrow_schema::DataType;

    if array.is_null(row) {
        return Value::Null;
    }

    match array.data_type() {
        DataType::Utf8 => {
            let a = array.as_string::<i32>();
            Value::String(a.value(row).to_string())
        }
        DataType::LargeUtf8 => {
            let a = array.as_string::<i64>();
            Value::String(a.value(row).to_string())
        }
        DataType::Boolean => {
            let a = array.as_boolean();
            Value::Bool(a.value(row))
        }
        DataType::Int8 => json_num_i64(
            array
                .as_primitive::<arrow_array::types::Int8Type>()
                .value(row) as i64,
        ),
        DataType::Int16 => json_num_i64(
            array
                .as_primitive::<arrow_array::types::Int16Type>()
                .value(row) as i64,
        ),
        DataType::Int32 => json_num_i64(
            array
                .as_primitive::<arrow_array::types::Int32Type>()
                .value(row) as i64,
        ),
        DataType::Int64 => json_num_i64(
            array
                .as_primitive::<arrow_array::types::Int64Type>()
                .value(row),
        ),
        DataType::UInt8 => json_num_u64(
            array
                .as_primitive::<arrow_array::types::UInt8Type>()
                .value(row) as u64,
        ),
        DataType::UInt16 => json_num_u64(
            array
                .as_primitive::<arrow_array::types::UInt16Type>()
                .value(row) as u64,
        ),
        DataType::UInt32 => json_num_u64(
            array
                .as_primitive::<arrow_array::types::UInt32Type>()
                .value(row) as u64,
        ),
        DataType::UInt64 => json_num_u64(
            array
                .as_primitive::<arrow_array::types::UInt64Type>()
                .value(row),
        ),
        DataType::Float32 => json_num_f64(
            array
                .as_primitive::<arrow_array::types::Float32Type>()
                .value(row) as f64,
        ),
        DataType::Float64 => json_num_f64(
            array
                .as_primitive::<arrow_array::types::Float64Type>()
                .value(row),
        ),
        DataType::List(_) | DataType::LargeList(_) => list_to_json(array, row),
        DataType::Struct(_) => struct_to_json(array, row),
        _ => Value::Null,
    }
}

fn json_num_i64(v: i64) -> Value {
    Value::Number(v.into())
}

fn json_num_u64(v: u64) -> Value {
    Value::Number(v.into())
}

fn json_num_f64(v: f64) -> Value {
    serde_json::Number::from_f64(v)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn list_to_json(array: &dyn arrow_array::Array, row: usize) -> Value {
    use arrow_array::cast::AsArray;
    use arrow_schema::DataType;
    match array.data_type() {
        DataType::List(_) => {
            let list = array.as_list::<i32>();
            let values = list.value(row);
            let mut out = Vec::with_capacity(values.len());
            for i in 0..values.len() {
                out.push(array_value_to_json(values.as_ref(), i));
            }
            Value::Array(out)
        }
        DataType::LargeList(_) => {
            let list = array.as_list::<i64>();
            let values = list.value(row);
            let mut out = Vec::with_capacity(values.len());
            for i in 0..values.len() {
                out.push(array_value_to_json(values.as_ref(), i));
            }
            Value::Array(out)
        }
        _ => Value::Null,
    }
}

fn struct_to_json(array: &dyn arrow_array::Array, row: usize) -> Value {
    use arrow_array::cast::AsArray;
    let s = array.as_struct();
    let mut map = serde_json::Map::new();
    for (i, field) in s.fields().iter().enumerate() {
        let col = s.column(i);
        map.insert(field.name().clone(), array_value_to_json(col.as_ref(), row));
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{ArrayRef, Int32Array, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;

    fn write_fixture(path: &std::path::Path) {
        let schema = Arc::new(Schema::new(vec![
            Field::new("title", DataType::Utf8, true),
            Field::new("count", DataType::Int32, true),
            Field::new(
                "proof",
                DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
                true,
            ),
        ]));
        let titles = StringArray::from(vec![Some("A"), Some("B")]);
        let counts = Int32Array::from(vec![Some(1), Some(2)]);

        let mut proof_builder =
            arrow_array::builder::ListBuilder::new(arrow_array::builder::StringBuilder::new());
        proof_builder.values().append_value("hash:abc");
        proof_builder.values().append_value("hash:def");
        proof_builder.append(true);
        proof_builder.values().append_value("hash:ghi");
        proof_builder.append(true);
        let proof_arr = proof_builder.finish();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(titles) as ArrayRef,
                Arc::new(counts) as ArrayRef,
                Arc::new(proof_arr) as ArrayRef,
            ],
        )
        .unwrap();
        let file = std::fs::File::create(path).unwrap();
        let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
    }

    #[test]
    fn streams_rows_from_local_parquet_fixture() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        write_fixture(tmp.path());

        let rows: Vec<_> = stream_parquet_rows(tmp.path()).unwrap().collect();
        assert_eq!(rows.len(), 2);

        let row0 = rows[0].as_ref().unwrap();
        assert_eq!(row0.get_str("title"), Some("A"));
        let proof = row0.get_array_strs("proof");
        assert_eq!(proof, vec!["hash:abc".to_string(), "hash:def".to_string()]);

        let row1 = rows[1].as_ref().unwrap();
        assert_eq!(row1.get_str("title"), Some("B"));
        assert_eq!(row1.get_array_strs("proof"), vec!["hash:ghi".to_string()]);
    }
}
