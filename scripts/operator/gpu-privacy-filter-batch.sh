#!/usr/bin/env bash
#
# Run the privacy filter on an ephemeral spot GPU for one batch, then stop.
#
# The pilot host serves the filter from a CPU shim on 127.0.0.1:8471 at roughly
# 58 characters/second. An L4 does the same work at ~43,000 -- measured, ~800x
# -- so a backlog that takes days on CPU is one short GPU session. There is no
# reason to keep a GPU running between sessions, so this script creates one,
# drains, and deletes it.
#
# HOW IT AVOIDS TOUCHING THE SERVER
#
# Ingest keeps pointing at 127.0.0.1:8471 throughout. `attach` stops the local
# CPU shim and opens an IAP TCP tunnel on that same port to the GPU box, so:
#
#   - no ingest config change, no restart, no code change;
#   - traffic is encrypted and authenticated by IAP rather than crossing the
#     VPC in plaintext (the self-hosted adapter has no TLS guard);
#   - no database credentials, KEK access or artifact keys ever reach the
#     ephemeral spot VM. It runs the stateless classifier only; ingest still
#     does envelope decrypt and release.
#
# USAGE
#   gpu-privacy-filter-batch.sh up       # create + provision the GPU box
#   gpu-privacy-filter-batch.sh attach   # local shim down, tunnel up
#   gpu-privacy-filter-batch.sh status   # held count and drain progress
#   gpu-privacy-filter-batch.sh detach   # tunnel down, local shim back up
#   gpu-privacy-filter-batch.sh down     # delete the GPU, VERIFY deletion
#
# `detach` and `down` are separate on purpose: detach restores the pilot to a
# working state on its own, so a failed teardown never leaves ingest without a
# filter.
set -euo pipefail

PROJECT="${TC_PROJECT:-tracecommons-pilot-2026}"
ZONE="${TC_ZONE:-us-central1-a}"
ACCOUNT="${TC_ACCOUNT:-zaki@iqlusion.io}"
GPU_VM="${TC_GPU_VM:-opf-gpu-drain}"
PILOT_VM="${TC_PILOT_VM:-tc-pilot-host}"
PORT=8471
TUNNEL_PIDFILE=/tmp/opf-gpu-tunnel.pid

G=(gcloud --account "$ACCOUNT" --project "$PROJECT")
say() { printf '\n=== %s\n' "$*"; }

cmd_up() {
  say "creating spot L4 ($GPU_VM)"
  # SPOT + DELETE termination: a preemption cleans itself up rather than
  # leaving a stopped GPU on the bill. Preemption mid-drain surfaces to ingest
  # as a transport error, which the adapter types as transient and does NOT
  # charge to the trace, so it is safe.
  "${G[@]}" compute instances create "$GPU_VM" \
    --zone "$ZONE" --machine-type g2-standard-4 \
    --provisioning-model=SPOT --instance-termination-action=DELETE \
    --maintenance-policy=TERMINATE \
    --image-family=common-cu129-ubuntu-2404-nvidia-580 \
    --image-project=deeplearning-platform-release \
    --boot-disk-size=80GB --boot-disk-type=pd-balanced \
    --metadata=install-nvidia-driver=True \
    --labels=ephemeral=true,purpose=opf-gpu-drain \
    --scopes=cloud-platform

  say "waiting for the GPU driver"
  until "${G[@]}" compute ssh "$GPU_VM" --zone "$ZONE" --tunnel-through-iap \
        --command "nvidia-smi -L" >/dev/null 2>&1; do sleep 20; done
  "${G[@]}" compute ssh "$GPU_VM" --zone "$ZONE" --tunnel-through-iap \
    --command "nvidia-smi --query-gpu=name,memory.total --format=csv,noheader"

  say "provisioning (a few minutes: CUDA torch + 2.7GB checkpoint)"
  "${G[@]}" compute ssh "$GPU_VM" --zone "$ZONE" --tunnel-through-iap --command '
    set -e
    # python3-dev is REQUIRED. Without it Triton fails to JIT CUDA kernels and
    # the real error ("Python.h: No such file or directory") is buried under a
    # CalledProcessError, which reads as a CUDA problem and is not one.
    sudo apt-get update -qq >/dev/null
    sudo apt-get install -y -qq python3-venv python3-dev build-essential >/dev/null
    python3 -m venv ~/opfgpu
    ~/opfgpu/bin/pip install -q --upgrade pip
    ~/opfgpu/bin/pip install -q torch --index-url https://download.pytorch.org/whl/cu129
    ~/opfgpu/bin/pip install -q "opf @ git+https://github.com/openai/privacy-filter.git" \
        huggingface_hub fastapi uvicorn pydantic
    ~/opfgpu/bin/python -c "import torch;assert torch.cuda.is_available();print(\"cuda ok\",torch.cuda.get_device_name(0))"
    # allow_patterns is REQUIRED. A full snapshot_download also pulls a ~10.5GB
    # onnx/ tree that nothing here uses; it took the pilot host to 98% disk.
    ~/opfgpu/bin/python - <<PY
from huggingface_hub import snapshot_download
import os
print("staged:", snapshot_download("openai/privacy-filter",
      allow_patterns=["original/*"], local_dir=os.path.expanduser("~/opfmodel")))
PY
  '

  say "installing the shim (same app.py the pilot runs, device=cuda)"
  "${G[@]}" compute scp deploy/pilot-gcp/privacy-filter/app.py \
    "$GPU_VM":~/app.py --zone "$ZONE" --tunnel-through-iap
  "${G[@]}" compute ssh "$GPU_VM" --zone "$ZONE" --tunnel-through-iap --command "
    export PRIVACY_FILTER_DEVICE=cuda
    export OPF_CHECKPOINT=\$HOME/opfmodel/original
    export TORCHDYNAMO_DISABLE=1
    setsid nohup \$HOME/opfgpu/bin/uvicorn app:app --host 127.0.0.1 --port $PORT \
      --workers 1 </dev/null > \$HOME/shim.log 2>&1 &
    sleep 25
    curl -s --max-time 30 localhost:$PORT/healthz; echo
  "
  say "GPU ready. Next: $0 attach"
}

cmd_attach() {
  say "stopping the pilot's local CPU shim (frees port $PORT)"
  "${G[@]}" compute ssh "$PILOT_VM" --zone "$ZONE" --tunnel-through-iap \
    --command "sudo systemctl stop trace-commons-privacy-filter && systemctl is-active trace-commons-privacy-filter || true"

  say "opening IAP tunnel  pilot:127.0.0.1:$PORT -> $GPU_VM:$PORT"
  # Run the tunnel FROM the pilot host so ingest keeps talking to loopback.
  "${G[@]}" compute ssh "$PILOT_VM" --zone "$ZONE" --tunnel-through-iap --command "
    setsid nohup gcloud compute start-iap-tunnel $GPU_VM $PORT \
      --local-host-port=127.0.0.1:$PORT --zone $ZONE --project $PROJECT \
      </dev/null > /tmp/opf-tunnel.log 2>&1 &
    echo \$! | sudo tee $TUNNEL_PIDFILE >/dev/null
    sleep 12
    echo -n 'tunnel healthz: '; curl -s --max-time 30 localhost:$PORT/healthz || echo FAILED
  "
  say "attached. ingest is unchanged and now served by the GPU."
}

cmd_status() {
  "${G[@]}" compute ssh "$PILOT_VM" --zone "$ZONE" --tunnel-through-iap --command '
    echo -n "held: "
    sudo bash -c "set -a; . /etc/tracecommons/ingest.env; set +a; psql \"\$TRACE_COMMONS_PII_BACKSTOP_DRIVER_DATABASE_URL\" -At -c \"SELECT count(*) FROM trace_submissions WHERE status = '"'"'awaiting_pii_backstop'"'"'\""
    echo "recent ticks:"
    sudo grep -a "PII backstop driver tick completed" /var/log/tracecommons/ingest.log \
      | sed "s/\x1b\[[0-9;]*m//g" | tail -3 | cut -c1-150
  '
}

cmd_detach() {
  say "closing the tunnel and restoring the local CPU shim"
  "${G[@]}" compute ssh "$PILOT_VM" --zone "$ZONE" --tunnel-through-iap --command "
    sudo pkill -f '[s]tart-iap-tunnel' || true
    sudo rm -f $TUNNEL_PIDFILE
    sudo systemctl start trace-commons-privacy-filter
    sleep 20
    echo -n 'local shim: '; systemctl is-active trace-commons-privacy-filter
    echo -n 'healthz: '; curl -s --max-time 60 localhost:$PORT/healthz || echo FAILED
  "
  say "detached. the pilot is self-sufficient again."
}

cmd_down() {
  say "deleting $GPU_VM"
  "${G[@]}" compute instances delete "$GPU_VM" --zone "$ZONE" --quiet
  # VERIFY. A verbal teardown is not a teardown: an unverified one cost $86 on
  # 2026-05-14.
  say "verifying deletion via the API"
  if "${G[@]}" compute instances describe "$GPU_VM" --zone "$ZONE" >/dev/null 2>&1; then
    echo "ERROR: $GPU_VM still exists" >&2; exit 1
  fi
  echo "$GPU_VM is gone."
  echo "orphaned disks:"
  "${G[@]}" compute disks list --filter='-users:*' --format='table(name,zone,sizeGb)'
}

case "${1:-}" in
  up) cmd_up ;;
  attach) cmd_attach ;;
  status) cmd_status ;;
  detach) cmd_detach ;;
  down) cmd_down ;;
  *) sed -n '2,30p' "$0"; exit 2 ;;
esac
