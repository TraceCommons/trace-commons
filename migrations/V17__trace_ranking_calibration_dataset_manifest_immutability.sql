-- Preserve hash-only holdout manifest identity for credit-bearing ranking evidence.

CREATE OR REPLACE FUNCTION reject_trace_ranking_calibration_dataset_manifest_update()
RETURNS trigger AS $$
BEGIN
    IF OLD.source_manifest_hash IS DISTINCT FROM NEW.source_manifest_hash
        OR OLD.source_count IS DISTINCT FROM NEW.source_count
        OR OLD.label_source_count IS DISTINCT FROM NEW.label_source_count
        OR OLD.label_actor_count IS DISTINCT FROM NEW.label_actor_count
    THEN
        RAISE EXCEPTION 'ranking calibration dataset manifest is immutable for this target use and policy'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trace_ranking_calibration_dataset_manifest_immutable
    ON trace_ranking_calibration_datasets;
CREATE TRIGGER trace_ranking_calibration_dataset_manifest_immutable
    BEFORE UPDATE OF source_manifest_hash, source_count, label_source_count, label_actor_count
    ON trace_ranking_calibration_datasets
    FOR EACH ROW
    EXECUTE FUNCTION reject_trace_ranking_calibration_dataset_manifest_update();
