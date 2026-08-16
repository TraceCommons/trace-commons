#!/usr/bin/env bash
#
# Provision a Windows dev box on GCE for building and RUNNING the WinUI
# contributor app.
#
# Why this exists: the WinUI project cannot be built anywhere but Windows, and
# CI can only tell you it compiles. Seeing the window actually render, and
# iterating on XAML without a CI round trip, needs a real Windows desktop.
#
# ACCESS IS OVER IAP, NOT A PUBLIC RDP PORT. The instance gets no external IP
# and 3389 is opened only to Google's IAP forwarding range. An internet-facing
# RDP port is a standing credential-stuffing target, and this project also
# hosts the pilot -- the blast radius of getting that wrong is not confined to
# a dev box.
#
# COST. Windows Server carries a per-vCPU license charge on top of compute, so
# this is not a free VM. Stop it when you are done:
#
#   ./provision-dev-vm.sh stop
#
# and verify the stop actually happened rather than assuming it did:
#
#   ./provision-dev-vm.sh status
#
set -euo pipefail

PROJECT="${TC_WIN_VM_PROJECT:-tracecommons-pilot-2026}"
ZONE="${TC_WIN_VM_ZONE:-us-central1-a}"
# Region is derived from the zone rather than configured separately, so the
# two cannot drift into a router that does not serve the instance's subnet.
REGION="${TC_WIN_VM_REGION:-${ZONE%-*}}"
ROUTER="${TC_WIN_VM_ROUTER:-tc-win-dev-router}"
NAT="${TC_WIN_VM_NAT:-tc-win-dev-nat}"
NAME="${TC_WIN_VM_NAME:-tc-win-dev}"
MACHINE="${TC_WIN_VM_MACHINE:-e2-standard-4}"
DISK_GB="${TC_WIN_VM_DISK_GB:-100}"
FIREWALL_RULE="allow-iap-rdp"

# Google's IAP TCP forwarding source range. Fixed and documented; the rule
# below must never be widened to 0.0.0.0/0.
IAP_RANGE="35.235.240.0/20"

usage() {
  cat <<'USAGE'
Usage: provision-dev-vm.sh <command>

  create    Create the instance and start the toolchain install
  status    Show the instance's current status (authoritative, via API)
  password  Reset and print the Windows password for the current user
  rdp       Open an IAP tunnel to 3389 on localhost
  start     Start a stopped instance
  stop      Stop the instance (disk still bills, compute does not)
  delete    Delete the instance and its boot disk
USAGE
}

require_gcloud() {
  command -v gcloud >/dev/null 2>&1 || {
    echo "gcloud is not installed" >&2
    exit 1
  }
}

# Preflight, every time. An expired credential should stop us here rather than
# halfway through creating billable resources.
preflight() {
  require_gcloud
  local account
  account="$(gcloud auth list --filter=status:ACTIVE --format='value(account)' 2>/dev/null || true)"
  if [ -z "$account" ]; then
    echo "No active gcloud account. Run: gcloud auth login" >&2
    exit 1
  fi
  echo "account: $account"
  echo "project: $PROJECT"
  echo "zone:    $ZONE"
}

ensure_firewall() {
  if gcloud compute firewall-rules describe "$FIREWALL_RULE" \
      --project "$PROJECT" >/dev/null 2>&1; then
    echo "firewall rule $FIREWALL_RULE already exists"
    return
  fi

  echo "creating firewall rule $FIREWALL_RULE (RDP from IAP range only)"
  gcloud compute firewall-rules create "$FIREWALL_RULE" \
    --project "$PROJECT" \
    --direction=INGRESS \
    --action=allow \
    --rules=tcp:3389 \
    --source-ranges="$IAP_RANGE" \
    --target-tags=tc-win-dev \
    --description="RDP to the Windows dev box, reachable only through IAP TCP forwarding"
}

# Egress for a VM with no external IP.
#
# --no-address is what keeps this box off the internet's RDP scanners, and it
# is not optional: this project's `default-allow-rdp` rule permits tcp:3389
# from 0.0.0.0/0 with NO target tags, so it applies to every instance in the
# network. Firewall rules are additive-allow, so a narrower rule cannot cancel
# it -- the only reliable defence is having no external address to reach.
#
# But a VM with no external IP also has no outbound internet, and the bootstrap
# has to download several GB of installers. Cloud NAT provides egress without
# providing ingress, which is exactly the asymmetry wanted here.
ensure_nat() {
  if ! gcloud compute routers describe "$ROUTER" \
      --project "$PROJECT" --region "$REGION" >/dev/null 2>&1; then
    echo "creating Cloud Router $ROUTER"
    gcloud compute routers create "$ROUTER" \
      --project "$PROJECT" \
      --region "$REGION" \
      --network default \
      --description="Router backing egress NAT for the Windows dev box"
  else
    echo "router $ROUTER already exists"
  fi

  if ! gcloud compute routers nats describe "$NAT" \
      --project "$PROJECT" --region "$REGION" --router "$ROUTER" >/dev/null 2>&1; then
    echo "creating Cloud NAT $NAT"
    gcloud compute routers nats create "$NAT" \
      --project "$PROJECT" \
      --region "$REGION" \
      --router "$ROUTER" \
      --auto-allocate-nat-external-ips \
      --nat-all-subnet-ip-ranges
  else
    echo "NAT $NAT already exists"
  fi
}

create() {
  preflight
  ensure_firewall
  ensure_nat

  echo "creating $NAME ($MACHINE, ${DISK_GB}GB)"
  # --no-address is the load-bearing flag: no external IP at all, so the only
  # route in is the IAP tunnel.
  gcloud compute instances create "$NAME" \
    --project "$PROJECT" \
    --zone "$ZONE" \
    --machine-type "$MACHINE" \
    --image-family windows-2022 \
    --image-project windows-cloud \
    --boot-disk-size "${DISK_GB}GB" \
    --boot-disk-type pd-balanced \
    --tags tc-win-dev \
    --no-address \
    --shielded-secure-boot \
    --shielded-vtpm \
    --shielded-integrity-monitoring \
    --metadata-from-file "windows-startup-script-ps1=$(dirname "$0")/win-dev-bootstrap.ps1"

  cat <<NEXT

Created. The startup script is now installing the toolchain, which takes a
while (Visual Studio Build Tools is the slow part). Watch it with:

  gcloud compute instances get-serial-port-output $NAME --project $PROJECT --zone $ZONE | tail -40

Then:
  ./provision-dev-vm.sh password    # set a password
  ./provision-dev-vm.sh rdp         # tunnel 3389 to localhost

NEXT
}

status() {
  preflight
  # Authoritative: ask the API rather than trusting what anyone remembers.
  gcloud compute instances describe "$NAME" \
    --project "$PROJECT" \
    --zone "$ZONE" \
    --format='table(name,status,machineType.basename(),lastStartTimestamp,lastStopTimestamp)'
}

password() {
  preflight
  gcloud compute reset-windows-password "$NAME" \
    --project "$PROJECT" \
    --zone "$ZONE" \
    --user "${TC_WIN_VM_USER:-$(whoami)}"
}

rdp() {
  preflight
  echo "Tunnelling 3389 -> localhost:13389. Connect an RDP client to localhost:13389."
  echo "Ctrl-C to close the tunnel."
  gcloud compute start-iap-tunnel "$NAME" 3389 \
    --local-host-port=localhost:13389 \
    --project "$PROJECT" \
    --zone "$ZONE"
}

start() {
  preflight
  gcloud compute instances start "$NAME" --project "$PROJECT" --zone "$ZONE"
  status
}

stop() {
  preflight
  gcloud compute instances stop "$NAME" --project "$PROJECT" --zone "$ZONE"
  # Re-read the state rather than reporting success from the stop command's
  # exit code. A verbal "it's stopped" has cost this project real money before.
  echo
  echo "Verified state after stop:"
  status
}

delete() {
  preflight
  gcloud compute instances delete "$NAME" \
    --project "$PROJECT" \
    --zone "$ZONE" \
    --delete-disks=all
}

case "${1:-}" in
  create) create ;;
  status) status ;;
  password) password ;;
  rdp) rdp ;;
  start) start ;;
  stop) stop ;;
  delete) delete ;;
  *) usage; exit 1 ;;
esac
