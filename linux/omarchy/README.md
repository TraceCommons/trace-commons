# Omarchy test box

One GCE instance, `tc-linux-omarchy`, for seeing the GTK shell
(`crates/trace-commons-contributor-gtk`) and its Omarchy skin on a real
Omarchy 4.x desktop (Arch Linux, Hyprland). It sits beside the GNOME and
KDE boxes (`linux/scripts/provision-dev-vm.sh`, on the `linux-mac-samples`
branch) and shares their hardening.

| | |
|---|---|
| Image | `arch-linux-gce`, family `arch` (arch-v20220910), upgraded in place |
| Desktop | Omarchy 4.5.0 (stable channel), Hyprland 0.56, SDDM autologin as `tc` |
| Machine | `e2-standard-4`, 100 GB pd-balanced, `us-central1-a`, pilot project |
| Rendering | llvmpipe on the GCE display device (simpledrm); no GPU |

## Cost

**It bills while running**, and its disk bills while stopped. Stop it when
you are done, then check `status` reads `TERMINATED`:

```bash
linux/scripts/provision-omarchy-vm.sh stop
linux/scripts/provision-omarchy-vm.sh status
```

## Security model

The same as the GNOME and KDE boxes:

- No external IP. SSH is reachable only through IAP TCP forwarding
  (`allow-iap-linux-dev`, source `35.235.240.0/20`); egress goes through the
  VPC's Cloud NAT. On the box, ufw denies incoming traffic except SSH from
  that range.
- No service account and no scopes.
- Tag `tc-linux-dev`, so `deny-linux-dev-internal-egress` denies it the
  private ranges. `create` refuses to run until both firewall rules exist;
  `provision-dev-vm.sh <gnome|kde> create` makes them.
- Public code only: clone the repository on the box. Never copy secrets or a
  contributor state directory onto it.
- No Shielded VM secure boot: the image declares no `SECURE_BOOT` guest
  feature and boots an unsigned systemd-boot.

## Usage

Set `TC_LINUX_VM_ACCOUNT` to your gcloud account if it is not the active one.

```bash
linux/scripts/provision-omarchy-vm.sh create      # stage 1: upgrade, then reboot
linux/scripts/provision-omarchy-vm.sh install     # stage 2: Omarchy, then reboot
linux/scripts/provision-omarchy-vm.sh status|start|stop|delete|serial
linux/scripts/provision-omarchy-vm.sh ssh "<command>"
linux/scripts/provision-omarchy-vm.sh screenshot out.png   # grim in tc's session
```

`create` sets `linux/scripts/omarchy-vm-bootstrap.sh` as the startup
script. Stage 1 moves the 2022 image onto Omarchy's pacman channel and
current Arch, then reboots. Stage 2 runs only once instance metadata
`tc-omarchy-install=yes` is set (`install` sets it). Every step writes a
marker under `/var/lib/tc-omarchy`, and the log is
`/var/log/tc-bootstrap.log`, ending in `tc bootstrap done`. `tc`'s password
is in `/root/tc-password`.

## How Omarchy is installed

`omarchy.org/install` no longer installs; Omarchy now ships from its ISO and
from `[omarchy] https://pkgs.omarchy.org/stable/$arch` with its Arch mirror
`stable-mirror.omarchy.org`. The bootstrap replays the ISO's install phases
(`omacom/omarchy-iso`, `orchestrator/phases_impl.py`) on the running VM:
packages, `useradd tc`, `omarchy-apply-system --install-user tc
--first-install`, `omarchy-provision-user --force --first-install` as `tc`,
then SDDM autologin. Omarchy's packaging key is fetched from its WKD and
only the pinned fingerprint `40DFB630FF42BCFFB047046CF0134EE680CAC571` is
trusted, as `bin/omarchy-update-keyring` does.

Where the box differs from a real install, all because the GCE image keeps
its own boot chain (UEFI systemd-boot on a 300 MB `/boot`, ext4 root):

- `limine`, `limine-mkinitcpio-hook` and `limine-snapper-sync` are passed as
  `--assume-installed`, so nothing writes Limine onto the boot partition.
- A snapper config is pre-seeded so the btrfs-only step does not fail on
  ext4; there are no snapshots, no disk encryption and no Plymouth.
- A last mkinitcpio drop-in keeps the image's `systemd modconf` hooks.
- NetworkManager replaces the image's dhclient, and the autologin is written
  by the bootstrap (the ISO writes it only for encrypted installs).

## Caveats

- **Never run a plain `pacman -Syu` or `omarchy-update` on the box.** An
  Omarchy upgrade pulls in Limine and would install it over the
  systemd-boot partition. Install single packages with `pacman -S --needed`
  and the three `--assume-installed` flags above.
- The bootstrap was fixed while it ran on 2026-11-17 and has not been re-run
  from scratch in its final order.
- Weston, which the nested-compositor tests use, was installed by hand
  afterwards; the bootstrap does not install it.
- In a non-login SSH shell `OMARCHY_PATH` is unset. `omarchy-theme-set`
  needs it, and takes the display name: `omarchy-theme-set "Catppuccin
  Latte"`.

## Running the shell in tc's session

Build as `tc` (`~/.cargo/bin/cargo`, with `RUSTFLAGS="-D warnings"
CARGO_INCREMENTAL=0`) from a clone of the repository, then run it in the
Hyprland session:

```bash
sudo -u tc bash -c 'cd ~
  export XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=wayland-1
  export DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus
  export HYPRLAND_INSTANCE_SIGNATURE=$(ls -t $XDG_RUNTIME_DIR/hypr | head -1)
  export OMARCHY_PATH=/usr/share/omarchy PATH=/usr/share/omarchy/bin:$PATH
  TC_SUPPRESS_ONBOARDING=1 TC_APPEARANCE_STATE=$HOME/work/appearance.json \
  TRACE_COMMONS_SAMPLE=normalDay setsid -f <checkout>/crates/trace-commons-contributor-gtk/target/debug/trace-commons-shell'
```

- `TRACE_COMMONS_SAMPLE` (debug builds, on the `linux-mac-samples` branch)
  puts the window on a recorded set. `TC_SUPPRESS_ONBOARDING=1` shows all
  four tabs and keeps the first-run window closed.
- `TC_APPEARANCE_STATE` points the Appearance choice at a scratch file
  (`{"appearance":"omarchy"}` or `"standard"`) instead of
  `~/.local/state/trace-commons/appearance.json`.
- Hyprland 0.56 takes Lua dispatchers, for example
  `hyprctl dispatch 'hl.dsp.focus({ window = "address:0x..." })'`,
  `hl.dsp.window.close(...)` and
  `hl.dsp.window.resize({ window = "address:0x...", x = -320, y = 0, relative = true })`.
- `wtype -M ctrl 3 -m ctrl` switches tabs (Ctrl+1 to Ctrl+4 are Home,
  Private AI, Traces, Insights); Ctrl+comma opens Settings as its own tiled
  window. A dropdown can be opened through AT-SPI (`python-gobject` and
  `at-spi2-core` are installed): its `toggle button` has a `click` action,
  after which arrow keys and Return choose an item.
- Omarchy's first-run notifications cover the top right corner of the
  screen; `omarchy-notification-dismiss "<summary>"` closes one.
- A single window takes the whole screen. Opening `foot` beside it gives a
  half-width tile, and a relative resize narrows it further.
