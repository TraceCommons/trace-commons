#!/usr/bin/env bash
# Startup script for the Linux dev boxes (see provision-dev-vm.sh). Runs as
# root on every boot; each step is idempotent, the package install runs once.
set -euo pipefail
exec > >(tee -a /var/log/tc-bootstrap.log) 2>&1
echo "== tc bootstrap $(date -u +%FT%TZ)"

FLAVOR="$(curl -fsS -H 'Metadata-Flavor: Google' \
  http://metadata.google.internal/computeMetadata/v1/instance/attributes/tc-desktop || echo gnome)"
export DEBIAN_FRONTEND=noninteractive

# The desktop user. Its password is only for RDP over the IAP tunnel.
if ! id tc >/dev/null 2>&1; then
  useradd -m -s /bin/bash -G sudo tc
  pw="$(head -c 18 /dev/urandom | base64 | tr -d '/+=')"
  echo "tc:$pw" | chpasswd
  umask 077; echo "$pw" > /root/tc-rdp-password
fi

COMMON=(build-essential pkg-config git curl ca-certificates libgtk-4-dev libadwaita-1-dev
  tesseract-ocr imagemagick weston dbus-user-session xdg-desktop-portal-gtk)
if [ ! -f /var/lib/tc-bootstrap.done ]; then
  apt-get update
  if [ "$FLAVOR" = kde ]; then
    apt-get install -y "${COMMON[@]}" kde-plasma-desktop sddm plasma-workspace krdp spectacle \
      xdg-desktop-portal-kde
  else
    apt-get install -y "${COMMON[@]}" ubuntu-desktop-minimal gnome-remote-desktop gnome-screenshot
  fi
  sudo -u tc -H bash -c 'curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal -c clippy,rustfmt'
  touch /var/lib/tc-bootstrap.done
fi

# Log the desktop user straight into a Wayland session and never lock it, so
# the session is always there for the app and for screenshots.
if [ "$FLAVOR" = kde ]; then
  mkdir -p /etc/sddm.conf.d
  cat > /etc/sddm.conf.d/tc-autologin.conf <<'EOF'
[Autologin]
User=tc
Session=plasma
Relogin=true
EOF
  install -d -o tc -g tc /home/tc/.config
  cat > /home/tc/.config/kscreenlockerrc <<'EOF'
[Daemon]
Autolock=false
LockOnResume=false
Timeout=0
EOF
  chown tc:tc /home/tc/.config/kscreenlockerrc
  systemctl set-default graphical.target
  systemctl enable sddm || true
else
  cat > /etc/gdm3/custom.conf <<'EOF'
[daemon]
AutomaticLoginEnable=true
AutomaticLogin=tc
WaylandEnable=true
EOF
  mkdir -p /etc/dconf/profile /etc/dconf/db/local.d
  printf 'user-db:user\nsystem-db:local\n' > /etc/dconf/profile/user
  cat > /etc/dconf/db/local.d/00-tc <<'EOF'
[org/gnome/desktop/screensaver]
lock-enabled=false
[org/gnome/desktop/session]
idle-delay=uint32 0
EOF
  dconf update
  systemctl set-default graphical.target
fi

if [ ! -f /var/lib/tc-bootstrap.rebooted ]; then
  touch /var/lib/tc-bootstrap.rebooted
  echo "== first boot complete, rebooting into the desktop"
  systemctl reboot
fi
echo "== tc bootstrap done"
