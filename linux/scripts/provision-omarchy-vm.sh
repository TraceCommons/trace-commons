#!/usr/bin/env bash
# Omarchy (Arch + Hyprland) test box for the GTK shell's Omarchy skin.
# Same hardening as linux/scripts/provision-dev-vm.sh (GNOME/KDE boxes):
# no external IP, reachable only through IAP TCP forwarding, no service
# account and no scopes, tag tc-linux-dev (reuses allow-iap-linux-dev and
# deny-linux-dev-internal-egress, both created by that script).
# Differences, each forced by the image:
#   - image projects/arch-linux-gce family arch (arch-v20220910, the newest
#     the builder published); no --shielded-secure-boot because the image
#     declares no SECURE_BOOT guest feature and boots unsigned systemd-boot;
#   - the startup script installs Omarchy in two stages, the second gated on
#     metadata tc-omarchy-install=yes (see omarchy-vm-bootstrap.sh).
#
# Usage: provision-omarchy-vm.sh <create|install|status|start|stop|delete|ssh|serial|screenshot [out.png]>
set -euo pipefail

CMD="${1:-}"
PROJECT="${TC_LINUX_VM_PROJECT:-tracecommons-pilot-2026}"
ZONE="${TC_LINUX_VM_ZONE:-us-central1-a}"
ACCOUNT="${TC_LINUX_VM_ACCOUNT:-}"
MACHINE="${TC_LINUX_VM_MACHINE:-e2-standard-4}"
DISK_GB="${TC_LINUX_VM_DISK_GB:-100}"
NAME=tc-linux-omarchy
TAG="tc-linux-dev"
HERE="$(cd "$(dirname "$0")" && pwd)"

g() {
  if [ -n "$ACCOUNT" ]; then gcloud --account="$ACCOUNT" --project="$PROJECT" "$@"; else gcloud --project="$PROJECT" "$@"; fi
}
require_firewall() {
  for r in allow-iap-linux-dev deny-linux-dev-internal-egress; do
    g compute firewall-rules describe "$r" >/dev/null 2>&1 || {
      echo "missing firewall rule $r: run linux/scripts/provision-dev-vm.sh gnome create first" >&2; exit 1; }
  done
}
# Commands run in tc's Hyprland session (grim, hyprctl).
SESSION_ENV='export XDG_RUNTIME_DIR=/run/user/$(id -u tc); export WAYLAND_DISPLAY=$(basename "$(ls $XDG_RUNTIME_DIR/wayland-* 2>/dev/null | grep -v lock | head -1)"); export HYPRLAND_INSTANCE_SIGNATURE=$(ls -t $XDG_RUNTIME_DIR/hypr 2>/dev/null | head -1)'

case "$CMD" in
  create)
    require_firewall
    g compute instances create "$NAME" --zone="$ZONE" --machine-type="$MACHINE" \
      --image-family=arch --image-project=arch-linux-gce \
      --boot-disk-size="${DISK_GB}GB" --boot-disk-type=pd-balanced \
      --no-address --no-service-account --no-scopes \
      --enable-display-device \
      --tags="$TAG" --labels=purpose=gtk-desktop-test,desktop=omarchy \
      --metadata=tc-desktop=omarchy,tc-omarchy-install=no \
      --metadata-from-file=startup-script="$HERE/omarchy-vm-bootstrap.sh"
    ;;
  install)
    g compute instances add-metadata "$NAME" --zone="$ZONE" --metadata=tc-omarchy-install=yes
    g compute ssh "$NAME" --zone="$ZONE" --tunnel-through-iap -- sudo systemctl start --no-block tc-omarchy-bootstrap.service
    ;;
  status) g compute instances describe "$NAME" --zone="$ZONE" --format='value(status)' ;;
  start) g compute instances start "$NAME" --zone="$ZONE" ;;
  stop) g compute instances stop "$NAME" --zone="$ZONE" ;;
  delete) g compute instances delete "$NAME" --zone="$ZONE" ;;
  serial) g compute instances get-serial-port-output "$NAME" --zone="$ZONE" ;;
  ssh) shift; g compute ssh "$NAME" --zone="$ZONE" --tunnel-through-iap -- "$@" ;;
  screenshot)
    out="${2:-omarchy-$(date +%Y%m%d-%H%M%S).png}"
    g compute ssh "$NAME" --zone="$ZONE" --tunnel-through-iap -- \
      "sudo -u tc bash -c '$SESSION_ENV; grim /tmp/tc-shot.png' && sudo cat /tmp/tc-shot.png" > "$out"
    echo "$out"
    ;;
  *) echo "usage: $0 <create|install|status|start|stop|delete|ssh|serial|screenshot [out.png]>" >&2; exit 2 ;;
esac
