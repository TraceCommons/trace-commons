# Retiring witness measurements after their rollback window

Every witness upgrade adds its new measurement beside the old ones, in three
places, so no client is broken by an upgrade it has not been told about
(`deploy/witness/README.md`, "Upgrades"). Step 6 of that procedure is to drop
the old measurements once every instance reports the new one and the rollback
window has passed. A pin set that only ever grows stops being a pin.

## Due: 2026-11-27

The 2026-11-12/13 upgrades left these entries on endpoint
`https://2904b99d74bc6ff9ffda211479b0a95cb9237a38-8088.dstack-pha1-prod5.phala.network`
(signer `0x85a17e34c30385ac2ba8cdc9981052a5857a2c11`, mrtd `2a674327...2b70`
throughout):

| Build | mrconfigid | Signed configuration entry `not_after` |
|---|---|---|
| `6abd2d127` | `03a5bf98bb048ea6...` | 2026-11-26 |
| `3644ca0de` | `03433bfd02ecae27...` | 2026-11-27 |
| `31d69f5d5` | `035d82d4968a01d2...` | 2026-11-27 |
| `ae40289cd` | `032a8f4fdd3fd79f...` | 2026-11-27 |
| `631c77be0` | `0387830e4954d8ce...` | 2026-11-27 |
| `035f862cd` (current at writing) | `03a3327ad93ca172...` | 2027-11-13, keep |

Read the full 96-hex values from the current signed configuration (served at
`/v1/public/witness-configuration` on ingest) rather than from this table.

## Before removing anything

1. Confirm what is running: `GET /v1/diagnostics` on the witness reports
   `build` and `witness_measurement`. Every instance must report the
   measurement you keep. Remove nothing that any running instance still
   reports.
2. Confirm the signed configuration's entries have expired as listed. An
   expired entry no longer authorizes routing on clients that hold a newer
   configuration, but ingest's certificate pins and the provisioning JSON are
   separate and must be edited explicitly.

## Remove

1. **Pilot ingest**, both pin lists, on `tc-pilot-host`, with the untracked
   helper `docs/operator/witness-redeploy-2026-11-05-pins.py` (copy it to the
   host first). It edits only the two variable definitions, backs up each
   touched file and prints a rollback line:

   ```bash
   sudo python3 -I ~/witness-redeploy-2026-11-05-pins.py show
   sudo python3 -I ~/witness-redeploy-2026-11-05-pins.py remove 'mrtd:<96hex>+mrconfigid:<96hex>'
   # once per retired measurement, then:
   sudo systemctl daemon-reload && sudo systemctl restart trace-commons-ingest
   curl -sS -o /dev/null -w 'health %{http_code}\n' http://127.0.0.1:3907/health
   ```

   Check `/v1/account/near/provision/capabilities` still answers `ready: true`
   with only the kept measurement(s).

2. **Signed witness configuration**: publish the next version with the
   retired entries removed, following `witness-configuration.md` "Publish"
   (canonicalize, sign with the owner KMS list key `tc-witness-list` /
   `witness-accepted-list` version 1, verify for each retained enrollment URL,
   install atomically at `TRACE_COMMONS_WITNESS_CONFIGURATION_PATH`, and check
   the public route serves identical bytes). Keep every `enrollment_urls`
   entry.

## Rollback

Each pin edit printed a `ROLLBACK:` line restoring the backed-up file; the
configuration install keeps the previous file beside it. Restore, restart
ingest, and check health.
