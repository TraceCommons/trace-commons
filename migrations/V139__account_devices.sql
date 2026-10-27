-- Account devices: let a contributor list, rename and revoke the devices
-- linked to their account.
--
-- `trace_sessions.device_key_id` names the device a native session was
-- minted for, so revoking one device ends that device's sessions and no
-- other. NULL for browser sessions and for sessions minted before this
-- migration; `revoke-all` still covers those.
--
-- `device_keys.label` is a name the contributor chose for the device. It is
-- shown back to them only and never recorded in audit rows or logs.

ALTER TABLE trace_sessions
    ADD COLUMN IF NOT EXISTS device_key_id TEXT
        CHECK (device_key_id IS NULL OR device_key_id ~ '^sha256:[0-9a-f]{64}$');

CREATE INDEX IF NOT EXISTS idx_trace_sessions_device_active
    ON trace_sessions (tenant_id, device_key_id)
    WHERE device_key_id IS NOT NULL AND revoked_at IS NULL;

ALTER TABLE device_keys
    ADD COLUMN IF NOT EXISTS label TEXT
        CHECK (label IS NULL OR char_length(label) BETWEEN 1 AND 64);
