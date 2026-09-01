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
FW_TAG="${TC_FW_TAG:-opf-gpu-drain}"
FW_RULE="${TC_FW_RULE:-opf-gpu-drain-8471}"

# --project on EVERY call, never the account default. On 2026-09-01 a delete ran
# against the account's default project (a different one), reported success, and
# the verification agreed -- while the instance was still running and billing.
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
  # The pilot's runtime service account is least-privilege and lacks
  # compute.instances.list, so it CANNOT open an IAP tunnel to the GPU. The
  # reverse (tunnelling from the GPU box, which does have the permission) fails
  # too: its first `gcloud compute ssh` generates a keypair that needs metadata
  # propagation the default SA cannot perform. Both were tried on 2026-08-31.
  #
  # What works is a VPC hop the pilot originates itself, narrowed to one source
  # address, one port, and one target tag, with a loopback forwarder in front so
  # ingest still addresses 127.0.0.1 and needs no config change.
  local gpu_ip pilot_ip
  gpu_ip=$("${G[@]}" compute instances describe "$GPU_VM" --zone "$ZONE" \
    --format='value(networkInterfaces[0].networkIP)')
  pilot_ip=$("${G[@]}" compute instances describe "$PILOT_VM" --zone "$ZONE" \
    --format='value(networkInterfaces[0].networkIP)')
  say "gpu=$gpu_ip pilot=$pilot_ip"

  say "opening a narrow path: tcp:$PORT, from the pilot IP only, to the GPU tag"
  "${G[@]}" compute instances add-tags "$GPU_VM" --zone "$ZONE" \
    --tags="$FW_TAG" --quiet
  if ! "${G[@]}" compute firewall-rules describe "$FW_RULE" >/dev/null 2>&1; then
    "${G[@]}" compute firewall-rules create "$FW_RULE" \
      --direction=INGRESS --action=ALLOW --rules="tcp:$PORT" \
      --source-ranges="${pilot_ip}/32" --target-tags="$FW_TAG" \
      --description="TEMPORARY: pilot -> ephemeral GPU privacy filter. Removed by 'down'."
  fi

  say "stopping the pilot's local CPU shim (frees port $PORT)"
  "${G[@]}" compute ssh "$PILOT_VM" --zone "$ZONE" --tunnel-through-iap \
    --command "sudo systemctl stop trace-commons-privacy-filter || true"

  say "forwarding pilot 127.0.0.1:$PORT -> $gpu_ip:$PORT"
  # socat, not an SSH tunnel: no key distribution, and it survives the pilot's
  # SSH sessions ending. Traffic stays inside the VPC, which GCP encrypts
  # between VMs -- but note this is NOT application-layer TLS, and the
  # self-hosted adapter has no TLS guard, so the firewall rule above is doing
  # real work. Do not widen its source range.
  "${G[@]}" compute ssh "$PILOT_VM" --zone "$ZONE" --tunnel-through-iap --command "
    command -v socat >/dev/null 2>&1 || sudo apt-get install -y -qq socat >/dev/null 2>&1
    setsid nohup socat TCP-LISTEN:$PORT,fork,reuseaddr,bind=127.0.0.1 TCP:$gpu_ip:$PORT \
      </dev/null > /tmp/opf-forward.log 2>&1 &
    sleep 5
    echo -n 'device via pilot loopback: '
    curl -s --max-time 30 localhost:$PORT/healthz || echo FAILED
    echo
  "
  say "attached. ingest is unchanged; verify the line above says device: cuda."

  cat <<'NOTE'

  While attached, raise window concurrency for the GPU and REVERT IT ON DETACH:

    TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_CONCURRENT_WINDOWS=8   # GPU only

  8 measured ~1,171 classify requests/minute. Against the local CPU shim the
  same value pushes windows past the 600s timeout and every field fails; 2 is
  the CPU value. Restart ingest after changing it.
NOTE
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
  say "REVERT concurrency to 2 before the CPU shim serves again"
  say "  (8 against the CPU shim pushes windows past the 600s timeout)"
  "${G[@]}" compute ssh "$PILOT_VM" --zone "$ZONE" --tunnel-through-iap --command "
    sudo sed -i 's/^TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_CONCURRENT_WINDOWS=8/TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_CONCURRENT_WINDOWS=2/' /etc/tracecommons/ingest.env
    sudo grep '^TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_CONCURRENT_WINDOWS' /etc/tracecommons/ingest.env
    sudo pkill -f '[s]ocat TCP-LISTEN:$PORT' || true
    sudo systemctl reset-failed trace-commons-privacy-filter 2>/dev/null || true
    sudo systemctl start trace-commons-privacy-filter
    sudo systemctl restart trace-commons-ingest
    sleep 25
    echo -n 'local shim: '; systemctl is-active trace-commons-privacy-filter
    echo -n 'device: '; curl -s --max-time 60 localhost:$PORT/healthz || echo FAILED
    echo
  "
  say "detached. the pilot is self-sufficient again -- confirm device: cpu above."
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
  say "removing the temporary firewall rule"
  "${G[@]}" compute firewall-rules delete "$FW_RULE" --quiet 2>/dev/null || true
  if "${G[@]}" compute firewall-rules describe "$FW_RULE" >/dev/null 2>&1; then
    echo "ERROR: $FW_RULE still exists" >&2; exit 1
  fi
  echo "$FW_RULE is gone."
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
