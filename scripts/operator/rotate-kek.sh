#!/usr/bin/env bash
#
# Stage a new Cloud KMS key version and validate via the key-rotation
# drill. Does NOT flip the previously-active version; the operator runs
# the disable command manually after the observation window.
#
# Flags:
#   --project=<gcp project>
#   --keyring=<keyring name>
#   --key=<key name>
#   --location=<location, e.g. us-central1>
#   --target=<base URL of running tracedao-ingest>
#   --admin-token=<bearer for the drill>

set -euo pipefail

PROJECT=""
KEYRING=""
KEY=""
LOCATION=""
TARGET=""
ADMIN_TOKEN=""

while [ $# -gt 0 ]; do
  case "$1" in
    --project=*)      PROJECT="${1#*=}";      shift ;;
    --keyring=*)      KEYRING="${1#*=}";      shift ;;
    --key=*)          KEY="${1#*=}";          shift ;;
    --location=*)     LOCATION="${1#*=}";     shift ;;
    --target=*)       TARGET="${1#*=}";       shift ;;
    --admin-token=*)  ADMIN_TOKEN="${1#*=}";  shift ;;
    *) echo "RotateKekUnknownArg: $1" >&2; exit 1 ;;
  esac
done

bail() { echo "RotateKekFailure: $1" >&2; exit 1; }

[ -n "$PROJECT" ]     || bail "project_unset"
[ -n "$KEYRING" ]     || bail "keyring_unset"
[ -n "$KEY" ]         || bail "key_unset"
[ -n "$LOCATION" ]    || bail "location_unset"
[ -n "$TARGET" ]      || bail "target_unset"
[ -n "$ADMIN_TOKEN" ] || bail "admin_token_unset"

command -v gcloud >/dev/null 2>&1 || bail "gcloud_not_installed"
command -v curl   >/dev/null 2>&1 || bail "curl_not_installed"
command -v jq     >/dev/null 2>&1 || bail "jq_not_installed"

# Snapshot the currently-primary version BEFORE rotation so the operator
# has a rollback handle.
PREV_PRIMARY=$(gcloud kms keys describe "$KEY" \
  --project="$PROJECT" --location="$LOCATION" --keyring="$KEYRING" \
  --format="value(primary.name)" | awk -F'/' '{print $NF}')

if [ -z "$PREV_PRIMARY" ]; then
  bail "previous_primary_unknown"
fi

echo "RotateKek: creating new version (was primary: $PREV_PRIMARY)"
gcloud kms keys versions create \
  --project="$PROJECT" \
  --location="$LOCATION" \
  --keyring="$KEYRING" \
  --key="$KEY" \
  --primary

NEW_PRIMARY=$(gcloud kms keys describe "$KEY" \
  --project="$PROJECT" --location="$LOCATION" --keyring="$KEYRING" \
  --format="value(primary.name)" | awk -F'/' '{print $NF}')

if [ "$NEW_PRIMARY" = "$PREV_PRIMARY" ]; then
  bail "primary_did_not_advance"
fi

echo "RotateKek: new primary = $NEW_PRIMARY"
echo "RotateKek: running key-rotation-drill"
code=$(curl -sS -o /tmp/rotation-drill.json -w "%{http_code}" -X POST \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  "$TARGET/v1/admin/key-rotation-drill")

if [ "$code" != "200" ]; then
  bail "drill_http_$code"
fi
ok=$(jq -r '.success // false' /tmp/rotation-drill.json)
if [ "$ok" != "true" ]; then
  bail "drill_not_success"
fi

cat <<EOF
RotateKekOK
new_primary=$NEW_PRIMARY
prev_primary=$PREV_PRIMARY

Rollback (re-point primary at prev version):
  gcloud kms keys versions update $PREV_PRIMARY \\
    --project=$PROJECT --location=$LOCATION \\
    --keyring=$KEYRING --key=$KEY --primary

Retire (only after the claim-lifetime window — see docs/operator/key-rotation.md):
  gcloud kms keys versions disable $PREV_PRIMARY \\
    --project=$PROJECT --location=$LOCATION \\
    --keyring=$KEYRING --key=$KEY
EOF
