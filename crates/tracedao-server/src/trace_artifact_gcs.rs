use bytes::Bytes;
use std::collections::BTreeMap;
use std::sync::Mutex;

pub struct GcsObjectFetch {
    pub body: Bytes,
    pub metadata: BTreeMap<String, String>,
}

pub trait GcsObjectClient: Send + Sync {
    fn put_object(
        &self,
        key: &str,
        body: Bytes,
        metadata: BTreeMap<String, String>,
    ) -> anyhow::Result<()>;
    fn get_object(&self, key: &str) -> anyhow::Result<GcsObjectFetch>;
    fn delete_object(&self, key: &str) -> anyhow::Result<bool>;
    fn restore_deleted_object(&self, key: &str) -> anyhow::Result<bool>;
}

#[derive(Default)]
pub struct InMemoryGcsObjectClient {
    live: Mutex<BTreeMap<String, (Bytes, BTreeMap<String, String>)>>,
    deleted: Mutex<BTreeMap<String, (Bytes, BTreeMap<String, String>)>>,
}

impl GcsObjectClient for InMemoryGcsObjectClient {
    fn put_object(
        &self,
        key: &str,
        body: Bytes,
        metadata: BTreeMap<String, String>,
    ) -> anyhow::Result<()> {
        self.live
            .lock()
            .unwrap()
            .insert(key.to_string(), (body, metadata));
        Ok(())
    }

    fn get_object(&self, key: &str) -> anyhow::Result<GcsObjectFetch> {
        let live = self.live.lock().unwrap();
        let (body, metadata) = live
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("GcsGetFailed: not found"))?;
        Ok(GcsObjectFetch {
            body: body.clone(),
            metadata: metadata.clone(),
        })
    }

    fn delete_object(&self, key: &str) -> anyhow::Result<bool> {
        if let Some(record) = self.live.lock().unwrap().remove(key) {
            self.deleted
                .lock()
                .unwrap()
                .insert(key.to_string(), record);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn restore_deleted_object(&self, key: &str) -> anyhow::Result<bool> {
        if let Some(record) = self.deleted.lock().unwrap().remove(key) {
            self.live.lock().unwrap().insert(key.to_string(), record);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
