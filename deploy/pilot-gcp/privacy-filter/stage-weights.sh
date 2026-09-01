#!/usr/bin/env bash
#
# Stage openai/privacy-filter weights to local disk.
#
# Run by the operator at deploy time, never by the service. The unit runs with
# HF_HUB_OFFLINE=1, so if the weights are not here the service fails to start
# rather than silently fetching them on a request path.
set -euo pipefail

DEST="${1:-/opt/tracecommons-privacy-filter/models/privacy_filter}"
REPO="openai/privacy-filter"
VENV="${VENV:-/opt/tracecommons-privacy-filter/venv}"

echo "Staging ${REPO} to ${DEST}"
mkdir -p "${DEST}"

"${VENV}/bin/python" - "$REPO" "$DEST" <<'PY'
import sys
from huggingface_hub import snapshot_download

repo, dest = sys.argv[1], sys.argv[2]
path = snapshot_download(repo_id=repo, local_dir=dest)
print(f"staged to {path}")
PY

# Refuse to report success on an empty directory. A staging step that "worked"
# but produced nothing is how a fail-closed boot turns into a 3am page.
shopt -s nullglob
weights=("${DEST}"/*.safetensors "${DEST}"/*/*.safetensors)
if [ ${#weights[@]} -eq 0 ]; then
    echo "ERROR: no .safetensors found under ${DEST} after staging" >&2
    exit 1
fi

echo "Staged ${#weights[@]} weight file(s)."
du -sh "${DEST}"
