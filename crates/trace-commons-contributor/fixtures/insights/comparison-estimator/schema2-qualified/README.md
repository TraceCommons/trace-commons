# Qualified schema 2 native fixture

`preview-response.json` is generated from the synthetic `qualified_spec` and
`qualified_result` helpers in `comparison_specs` on this implementation branch.
The Rust regression requires exact JSON-value equality with the producer and
validates the result against its immutable specification. Native bridge tests
consume the same response. It contains no imported or private source data.
