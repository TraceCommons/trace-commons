ALTER TABLE trace_revocation_propagation_items
    DROP CONSTRAINT IF EXISTS trace_revocation_propagation_items_target_kind_check;

ALTER TABLE trace_revocation_propagation_items
    ADD CONSTRAINT trace_revocation_propagation_items_target_kind_check
    CHECK (
        target_kind IN (
            'object_ref',
            'export_manifest',
            'export_manifest_item',
            'vector_entry',
            'derived_record',
            'benchmark_artifact',
            'ranker_artifact',
            'credit_settlement',
            'worker_queue',
            'physical_delete_receipt'
        )
    );

ALTER TABLE trace_revocation_propagation_items
    DROP CONSTRAINT IF EXISTS trace_revocation_propagation_items_action_check;

ALTER TABLE trace_revocation_propagation_items
    ADD CONSTRAINT trace_revocation_propagation_items_action_check
    CHECK (
        action IN (
            'invalidate_metadata',
            'invalidate_export_membership',
            'invalidate_vector',
            'invalidate_benchmark_artifact',
            'invalidate_ranker_artifact',
            'reverse_credit_settlement',
            'invalidate_worker_queue',
            'delete_object_payload',
            'record_physical_delete_receipt'
        )
    );
