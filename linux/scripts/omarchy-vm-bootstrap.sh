#!/usr/bin/env bash
# Startup script for tc-linux-omarchy (see provision-omarchy-vm.sh): turns the
# public Arch GCE image (arch-linux-gce, family arch; last built 2022-09-10)
# into an Omarchy (stable channel) Hyprland desktop for user tc, auto-logged in,
# software-rendered, reachable only over IAP SSH.
#
# Omarchy installs only from its ISO since June 2026 (omarchy.org/install now
# just prints that). This script replays the ISO's target phases on the
# running VM instead (omarchy-iso orchestrator/phases_impl.py):
#   packages -> useradd -> omarchy-apply-system --install-user tc --first-install
#   -> omarchy-provision-user --force --first-install (as tc) -> sddm login.
# Deviations, all because the GCE image keeps its own boot chain (UEFI
# systemd-boot on a 300 MB vfat /boot, ext4 root):
#   - limine, limine-mkinitcpio-hook, limine-snapper-sync are --assume-installed
#     so pacman never installs Limine over the ESP; snapper config is
#     pre-seeded so snapper.sh does not fail on ext4;
#   - a last mkinitcpio drop-in restores the GCE image's HOOKS line, since
#     Omarchy's baseline adds encrypt/btrfs-overlayfs/plymouth for LUKS+btrfs;
#   - ufw (default deny incoming, from Omarchy firewall.sh) gets an SSH
#     allowance from the IAP range only;
#   - NetworkManager replaces the image's dhclient@eth0.
# Stage 1 (system upgrade onto Omarchy's pacman channel) always runs. Stage 2
# (Omarchy itself) runs only when instance metadata tc-omarchy-install=yes, so
# the display stack can be probed in between. Each step has a done-marker; the
# script runs on every boot, and also from its own unit (tc-omarchy-bootstrap)
# so it does not depend on the 2022 guest agent surviving the upgrade.
set -euo pipefail
exec 9>/run/tc-omarchy-bootstrap.lock
flock -n 9 || { echo "tc bootstrap already running"; exit 0; }
exec > >(tee -a /var/log/tc-bootstrap.log >/dev/console) 2>&1
echo "== tc omarchy bootstrap $(date -u +%FT%TZ)"
ST=/var/lib/tc-omarchy; mkdir -p "$ST"
md() { curl -fsS -H 'Metadata-Flavor: Google' "http://169.254.169.254/computeMetadata/v1/$1" 2>/dev/null || true; }
step() { [ -f "$ST/$1.done" ] && { echo "-- $1: done"; return 1; }; echo "-- $1: start $(date -u +%T)"; return 0; }
done_() { touch "$ST/$1.done"; echo "-- $1: done $(date -u +%T)"; }
IAP_RANGE=35.235.240.0/20
# On the first boot of 2026-11-17 every pacman download timed out in the first
# minute after boot (Cloud NAT not yet passing traffic); wait for egress.
for i in $(seq 1 60); do
  curl -4 -fsS -m 10 -o /dev/null https://stable-mirror.omarchy.org/core/os/x86_64/core.db && break
  echo "waiting for egress ($i)"; sleep 10
done
# pacman with retries, for transient mirror/NAT failures.
pac() { local n; for n in 1 2 3; do pacman "$@" && return 0; echo "pacman failed (try $n), retrying"; sleep 20; done; return 1; }

# Run from our own unit too, so a broken guest agent cannot strand later stages.
install -m 0755 "$0" /usr/local/sbin/tc-omarchy-bootstrap.sh 2>/dev/null || true
if [ ! -f /etc/systemd/system/tc-omarchy-bootstrap.service ]; then
  cat > /etc/systemd/system/tc-omarchy-bootstrap.service <<'EOF'
[Unit]
Description=tc-linux-omarchy bootstrap (idempotent)
After=network-online.target
Wants=network-online.target
[Service]
Type=oneshot
ExecStart=/usr/local/sbin/tc-omarchy-bootstrap.sh
TimeoutStartSec=infinity
[Install]
WantedBy=multi-user.target
EOF
  systemctl enable tc-omarchy-bootstrap.service
fi

# Fallback SSH access: copy the metadata ssh-keys into authorized_keys on every
# boot, independent of the guest agent (users get wheel + passwordless sudo,
# as the guest agent's google-sudoers would give them).
cat > /usr/local/sbin/tc-ssh-keys.sh <<'EOF'
#!/usr/bin/env bash
set -u
keys="$( { curl -fsS -H 'Metadata-Flavor: Google' http://169.254.169.254/computeMetadata/v1/instance/attributes/ssh-keys; echo;
  curl -fsS -H 'Metadata-Flavor: Google' http://169.254.169.254/computeMetadata/v1/project/attributes/ssh-keys; echo; } 2>/dev/null)"
printf '%s\n' "$keys" | grep -E '^[a-z_][a-z0-9_-]*:(ssh-|ecdsa-)' | cut -d: -f1 | sort -u | while read -r u; do
  [ "$u" = root ] && continue
  id "$u" >/dev/null 2>&1 || useradd -m -G wheel "$u"
  h=$(getent passwd "$u" | cut -d: -f6); install -d -m 700 -o "$u" -g "$u" "$h/.ssh"
  printf '%s\n' "$keys" | grep "^$u:" | cut -d: -f2- | sed 's/ google-ssh.*//' | sort -u > "$h/.ssh/authorized_keys.tc"
  install -m 600 -o "$u" -g "$u" "$h/.ssh/authorized_keys.tc" "$h/.ssh/authorized_keys"; rm -f "$h/.ssh/authorized_keys.tc"
  echo "$u ALL=(ALL) NOPASSWD: ALL" > "/etc/sudoers.d/90-tc-gce-$u"; chmod 440 "/etc/sudoers.d/90-tc-gce-$u"
done
EOF
chmod 755 /usr/local/sbin/tc-ssh-keys.sh
cat > /etc/systemd/system/tc-ssh-keys.service <<'EOF'
[Unit]
Description=Install GCE metadata ssh-keys (guest-agent fallback)
After=network-online.target
Wants=network-online.target
[Service]
Type=oneshot
ExecStart=/usr/local/sbin/tc-ssh-keys.sh
[Install]
WantedBy=multi-user.target
EOF
systemctl enable tc-ssh-keys.service >/dev/null 2>&1 || true
/usr/local/sbin/tc-ssh-keys.sh || true

# ---------------------------------------------------------------- stage 1
if step grow-root; then
  # The image is 10 GB on a 100 GB disk. Move the backup GPT to the end, grow
  # partition 2 (root), then the ext4 filesystem, online.
  disk=/dev/sda
  sfdisk --relocate gpt-bak-std "$disk" || true
  echo ', +' | sfdisk --no-reread --force -N 2 "$disk" || true
  partx -u "$disk" || true
  resize2fs "${disk}2" || true
  df -h /
  done_ grow-root
fi

if step pacman-omarchy; then
  # Omarchy's stable channel (default/pacman/pacman-stable.conf and
  # mirrorlist-stable at omacom/omarchy quattro 3437cfb4). This also drops the
  # image's dead [gce] repo (storage.googleapis.com/arch-linux-gce/repo: 403).
  cp -n /etc/pacman.conf /etc/pacman.conf.gce-orig || true
  cat > /etc/pacman.conf <<'EOF'
[options]
Color
VerbosePkgLists
HoldPkg = pacman glibc
Architecture = auto
CheckSpace
ParallelDownloads = 5
SigLevel = Required DatabaseOptional
LocalFileSigLevel = Optional

[omarchy]
Server = https://pkgs.omarchy.org/stable/$arch

[core]
Include = /etc/pacman.d/mirrorlist

[extra]
Include = /etc/pacman.d/mirrorlist

[multilib]
Include = /etc/pacman.d/mirrorlist
EOF
  cp -n /etc/pacman.d/mirrorlist /etc/pacman.d/mirrorlist.gce-orig || true
  echo 'Server = https://stable-mirror.omarchy.org/$repo/os/$arch' > /etc/pacman.d/mirrorlist
  done_ pacman-omarchy
fi

if step keyrings; then
  # Start from a clean keyring: the image's 2022 one trusts retired keys, and
  # an interrupted earlier run leaves a trustdb --init refuses to update.
  rm -rf /etc/pacman.d/gnupg
  pacman-key --init
  pacman-key --populate archlinux || true
  # The current archlinux-keyring verified against the image's 2022 keys on
  # 2026-11-17, so no unsigned fallback.
  pac -Sy --noconfirm archlinux-keyring
  pacman-key --populate archlinux
  done_ keyrings
fi

if step upgrade; then
  # A four-year jump. pacman first (7.x adds the alpm download user), then all.
  # Expect one "error: command failed to execute correctly" from the dbus
  # reload hook (the old dbus is still running); the reboot below clears it.
  pacman -Sy --noconfirm --needed pacman || true
  pac -Syu --noconfirm --overwrite '*'
  done_ upgrade
fi

if step network; then
  # Hand eth0 to NetworkManager (what Omarchy enables) instead of the image's
  # dhclient@eth0, which the upgrade leaves without a maintained package.
  pac -S --noconfirm --needed networkmanager openssh sudo
  systemctl enable NetworkManager.service sshd.service systemd-resolved.service
  systemctl disable dhclient@eth0.service 2>/dev/null || true
  ln -sf /run/systemd/resolve/stub-resolv.conf /etc/resolv.conf.tc-pending
  echo '%wheel ALL=(ALL:ALL) ALL' > /etc/sudoers.d/10-wheel; chmod 440 /etc/sudoers.d/10-wheel
  done_ network
fi

if [ ! -f "$ST/stage1-rebooted" ]; then
  touch "$ST/stage1-rebooted"
  mv -f /etc/resolv.conf.tc-pending /etc/resolv.conf
  sync; echo "== stage 1 complete, rebooting into the upgraded system"
  systemctl reboot; exit 0
fi

# ---------------------------------------------------------------- stage 2
if [ "$(md instance/attributes/tc-omarchy-install)" != yes ]; then
  echo "== stage 2 gated: set metadata tc-omarchy-install=yes to install Omarchy"
  exit 0
fi

# On the 2026-11-17 run this step ran here, after the stage 1 upgrade and reboot.
if step omarchy-key; then
  # Omarchy's packaging key, as bin/omarchy-update-keyring adds it: fetched from
  # Omarchy's own WKD for pkgs@omarchy.org, and only the pinned fingerprint is
  # added and locally signed. The same fingerprint ships in omarchy-keyring
  # (omacom/omarchy-pkgs pkgbuilds/omarchy-keyring/omarchy-trusted).
  # An earlier version of this script lsigned 68B3537F..., which is the Arch
  # Linux ARM build key in omarchy's default/pacman/keys/; drop it.
  pacman-key --delete 68B3537F39A313B3E574D06777193F152BDBE6A6 2>/dev/null || true
  rm -f /root/omarchy-packaging.asc
  fpr=40DFB630FF42BCFFB047046CF0134EE680CAC571
  (
    GNUPGHOME=$(mktemp -d); export GNUPGHOME
    trap 'gpgconf --kill all 2>/dev/null || true; rm -rf "$GNUPGHOME"' EXIT
    gpg --batch --quiet --auto-key-locate clear,nodefault,wkd --locate-external-keys pkgs@omarchy.org
    gpg --batch --export "$fpr" > "$GNUPGHOME/omarchy.gpg"
    pacman-key --add "$GNUPGHOME/omarchy.gpg"
  )
  pacman-key --finger "$fpr"
  pacman-key --lsign-key "$fpr"
  done_ omarchy-key
fi

NOLIMINE=(--assume-installed limine --assume-installed limine-mkinitcpio-hook
  --assume-installed limine-snapper-sync)

if step mkinitcpio-gce; then
  # Keep the GCE image's initramfs: Omarchy's baseline HOOKS add encrypt,
  # btrfs-overlayfs and plymouth for its LUKS+btrfs+Limine layout. Sorts last.
  mkdir -p /etc/mkinitcpio.conf.d
  cat > /etc/mkinitcpio.conf.d/zz-tc-gce.conf <<'EOF'
# tc-linux-omarchy: GCE boot chain (systemd-boot, ext4 root by LABEL=root).
MODULES=(virtio_pci virtio_scsi sd_mod ext4)
BINARIES=(fsck fsck.ext4)
HOOKS=(systemd modconf)
EOF
  done_ mkinitcpio-gce
fi

if step omarchy-packages; then
  pac -S --noconfirm --needed "${NOLIMINE[@]}" omarchy-keyring omarchy-settings
  pac -S --noconfirm --needed "${NOLIMINE[@]}" omarchy omarchy-nvim
  # The ISO's core package set, from the installed (stable) omarchy package.
  mapfile -t pkgs < <(cat /usr/share/omarchy/install/omarchy-base.packages \
    /usr/share/omarchy/install/omarchy-x86_64-only.packages 2>/dev/null | sed 's/#.*//' | tr -s ' \t' '\n' | grep -v '^$')
  echo "base packages: ${#pkgs[@]}"
  pac -S --noconfirm --needed "${NOLIMINE[@]}" "${pkgs[@]}"
  # The GTK shell's build and test dependencies.
  pac -S --noconfirm --needed "${NOLIMINE[@]}" base-devel pkgconf git gtk4 libadwaita \
    grim slurp mesa vulkan-swrast tesseract tesseract-data-eng imagemagick xdg-desktop-portal-gtk
  done_ omarchy-packages
fi

if step initramfs; then
  mkinitcpio -P
  ls -la /boot/initramfs-linux.img /boot/vmlinuz-linux /boot/loader/entries/
  done_ initramfs
fi

if step user-tc; then
  # After the packages, so useradd copies the complete /etc/skel (as the ISO).
  if ! id tc >/dev/null 2>&1; then
    useradd -m -s /bin/bash -G wheel tc
    pw="$(head -c 18 /dev/urandom | base64 | tr -d '/+=')"
    echo "tc:$pw" | chpasswd
    umask 077; echo "$pw" > /root/tc-password
  fi
  done_ user-tc
fi

if step apply-system; then
  # snapper.sh runs `snapper create-config /`, which needs btrfs; a present
  # config skips it, and the template is then installed over it.
  mkdir -p /etc/snapper/configs; [ -f /etc/snapper/configs/root ] || : > /etc/snapper/configs/root
  /usr/bin/omarchy-apply-system --install-user tc --first-install
  done_ apply-system
fi

if step provision-user; then
  sudo -u tc -H env -C /home/tc /usr/bin/omarchy-provision-user --force --first-install
  done_ provision-user
fi

if step gce-fixups; then
  # SSH through IAP only (the VPC firewall already limits port 22 to it).
  # With ufw inactive (it starts at boot), `ufw allow` saves the rule and then
  # prints "ERROR: problem running" (seen 2026-11-17), so check the saved rule.
  ufw allow from "$IAP_RANGE" to any port 22 proto tcp comment 'IAP SSH' || true
  ufw show added | grep -qF "ufw allow from $IAP_RANGE to any port 22 proto tcp"
  systemctl enable sshd.service NetworkManager.service
  systemctl disable dhclient@eth0.service 2>/dev/null || true
  # Initramfs again: apply-system's hardware steps may have changed inputs.
  mkinitcpio -P
  # Autologin into the Omarchy session (the ISO writes this for encrypted
  # installs; this box has no LUKS prompt and no person at the console).
  mkdir -p /etc/sddm.conf.d
  printf '[Autologin]\nUser=tc\nSession=omarchy.desktop\nRelogin=true\n' > /etc/sddm.conf.d/autologin.conf
  printf '[Theme]\nCurrent=omarchy\n\n[Users]\nRememberLastUser=true\nRememberLastSession=true\n' > /etc/sddm.conf.d/99-omarchy-login.conf
  mkdir -p /var/lib/sddm; printf '[Last]\nSession=omarchy.desktop\nUser=tc\n' > /var/lib/sddm/state.conf
  chown -R sddm:sddm /var/lib/sddm 2>/dev/null || true
  ls -la /usr/local/share/wayland-sessions/ /usr/share/wayland-sessions/ 2>/dev/null || true
  # Never lock, sleep or blank: the session must stay visible for screenshots.
  # omarchy-toggle-idle stay-awake writes ~/.local/state/omarchy/indicators/stay-awake;
  # the toggles/ flags are named for the off state (manual 13).
  sudo -u tc mkdir -p /home/tc/.local/state/omarchy/indicators /home/tc/.local/state/omarchy/toggles
  sudo -u tc touch /home/tc/.local/state/omarchy/indicators/stay-awake \
    /home/tc/.local/state/omarchy/toggles/screensaver-off /home/tc/.local/state/omarchy/toggles/suspend-off
  systemctl set-default graphical.target
  systemctl enable sddm.service
  done_ gce-fixups
fi

if step rustup; then
  sudo -u tc -H bash -c 'curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal -c clippy,rustfmt'
  done_ rustup
fi

if [ ! -f "$ST/stage2-rebooted" ]; then
  touch "$ST/stage2-rebooted"
  sync; echo "== stage 2 complete, rebooting into the Omarchy session"
  systemctl reboot; exit 0
fi
echo "== tc omarchy bootstrap done"
