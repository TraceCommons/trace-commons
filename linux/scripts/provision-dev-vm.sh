#!/usr/bin/env bash
# Linux desktop test boxes for the GTK shell: one GNOME (Ubuntu 24.04) and
# one KDE Plasma 6 (Debian 13), both Wayland, software-rendered.
#
# Mirrors windows/scripts/provision-dev-vm.sh: no external IP, reachable only
# through IAP TCP forwarding, egress through the existing Cloud NAT.
# Hardening beyond the Windows box, because these share the pilot's VPC:
#   - no service account and no scopes: the VM holds no credential for the
#     pilot project's KMS, buckets or database;
#   - an egress rule denies the VMs any traffic to the VPC's internal ranges,
#     so they cannot reach the pilot or staging hosts through the
#     default-allow-internal rule.
#
# Usage: provision-dev-vm.sh <gnome|kde> <create|status|start|stop|delete|ssh|rdp>
set -euo pipefail

FLAVOR="${1:-}"
CMD="${2:-}"
PROJECT="${TC_LINUX_VM_PROJECT:-tracecommons-pilot-2026}"
ZONE="${TC_LINUX_VM_ZONE:-us-central1-a}"
ACCOUNT="${TC_LINUX_VM_ACCOUNT:-}"
MACHINE="${TC_LINUX_VM_MACHINE:-e2-standard-4}"
DISK_GB="${TC_LINUX_VM_DISK_GB:-100}"
TAG="tc-linux-dev"
IAP_RANGE="35.235.240.0/20"
HERE="$(cd "$(dirname "$0")" && pwd)"

case "$FLAVOR" in
  gnome) NAME=tc-linux-gnome; IMAGE_FAMILY=ubuntu-2404-lts-amd64; IMAGE_PROJECT=ubuntu-os-cloud ;;
  kde) NAME=tc-linux-kde; IMAGE_FAMILY=debian-13; IMAGE_PROJECT=debian-cloud ;;
  *) echo "usage: $0 <gnome|kde> <create|status|start|stop|delete|ssh|rdp>" >&2; exit 2 ;;
esac

g() {
  if [ -n "$ACCOUNT" ]; then gcloud --account="$ACCOUNT" --project="$PROJECT" "$@"; else gcloud --project="$PROJECT" "$@"; fi
}

ensure_firewall() {
  if ! g compute firewall-rules describe allow-iap-linux-dev >/dev/null 2>&1; then
    g compute firewall-rules create allow-iap-linux-dev --direction=INGRESS --action=allow \
      --rules=tcp:22,tcp:3389 --source-ranges="$IAP_RANGE" --target-tags="$TAG" \
      --description="SSH and RDP to the Linux dev boxes, only through IAP TCP forwarding"
  fi
  if ! g compute firewall-rules describe deny-linux-dev-internal-egress >/dev/null 2>&1; then
    g compute firewall-rules create deny-linux-dev-internal-egress --direction=EGRESS --action=deny \
      --rules=all --destination-ranges=10.0.0.0/8,172.16.0.0/12,192.168.0.0/16 \
      --target-tags="$TAG" --priority=900 \
      --description="Linux dev boxes may not reach other hosts in this VPC (pilot, staging)"
  fi
}

case "$CMD" in
  create)
    ensure_firewall
    g compute instances create "$NAME" --zone="$ZONE" --machine-type="$MACHINE" \
      --image-family="$IMAGE_FAMILY" --image-project="$IMAGE_PROJECT" \
      --boot-disk-size="${DISK_GB}GB" --boot-disk-type=pd-balanced \
      --no-address --no-service-account --no-scopes \
      --enable-display-device --shielded-secure-boot \
      --tags="$TAG" --labels=purpose=gtk-desktop-test,desktop="$FLAVOR" \
      --metadata=tc-desktop="$FLAVOR" \
      --metadata-from-file=startup-script="$HERE/linux-dev-bootstrap.sh"
    ;;
  status) g compute instances describe "$NAME" --zone="$ZONE" --format='value(status)' ;;
  start) g compute instances start "$NAME" --zone="$ZONE" ;;
  stop) g compute instances stop "$NAME" --zone="$ZONE" ;;
  delete) g compute instances delete "$NAME" --zone="$ZONE" ;;
  ssh) shift 2; g compute ssh "$NAME" --zone="$ZONE" --tunnel-through-iap -- "$@" ;;
  rdp)
    port="${TC_LINUX_VM_RDP_PORT:-13389}"
    echo "RDP on localhost:$port (user tc; password: sudo cat /root/tc-rdp-password on the box)"
    g compute start-iap-tunnel "$NAME" 3389 --local-host-port="localhost:$port" --zone="$ZONE"
    ;;
  *) echo "usage: $0 <gnome|kde> <create|status|start|stop|delete|ssh|rdp>" >&2; exit 2 ;;
esac
