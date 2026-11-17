# Linux desktop test boxes

Two GCE instances for seeing the GTK shell
(`crates/trace-commons-contributor-gtk`) on real Linux desktops, driven
from a machine that is not running Linux. They mirror the Windows dev VM
(`windows/docs/dev-vm.md`).

| Box | Image | Desktop |
|---|---|---|
| `tc-linux-gnome` | Ubuntu 24.04 | GNOME, Wayland |
| `tc-linux-kde` | Debian 13 | Plasma 6, Wayland |

Both are `e2-standard-4` with a 100 GB disk, in `us-central1-a` of the
pilot project, and render in software (no GPU). The bootstrap sets the
desktop user `tc` to log straight into a Wayland session that never locks.
Check that the session exists (`loginctl list-sessions`) before relying on
it: on 2026-11-17 the KDE box had none. Without it, run the app under a
nested headless Weston as your own user (see Screenshots).

## Cost

**They bill while running.** Each box bills for its compute every hour it is
up, and its disk bills even while stopped. Stop a box when you are done:

```bash
linux/scripts/provision-dev-vm.sh gnome stop
linux/scripts/provision-dev-vm.sh kde stop
```

Then check `status` reads `TERMINATED`.

## Security model

- **No external IP.** SSH (22) and RDP (3389) are reachable only through IAP
  TCP forwarding (`allow-iap-linux-dev`, source `35.235.240.0/20`). Egress
  goes through the VPC's existing Cloud NAT. Do not attach an external IP to
  get egress back; fix the NAT instead (see the Windows doc).
- **No service account, no scopes.** The boxes share the pilot's VPC and
  project, so they hold no credential at all: nothing for the pilot's KMS,
  buckets or database.
- **No internal egress.** `deny-linux-dev-internal-egress` denies the boxes
  any traffic to `10.0.0.0/8`, `172.16.0.0/12` and `192.168.0.0/16`, so they
  cannot reach the pilot or staging hosts through `default-allow-internal`.
- **Public code only.** The repository is public: clone it on the box and
  fetch your pushed branch. Never copy secrets, a contributor state
  directory or anything else from your own machine onto a box.

## Usage

Set `TC_LINUX_VM_ACCOUNT` to your gcloud account if it is not the active one.

```bash
linux/scripts/provision-dev-vm.sh <gnome|kde> create   # firewall rules, then the box
linux/scripts/provision-dev-vm.sh <gnome|kde> status
linux/scripts/provision-dev-vm.sh <gnome|kde> start
linux/scripts/provision-dev-vm.sh <gnome|kde> stop
linux/scripts/provision-dev-vm.sh <gnome|kde> ssh "<command>"
linux/scripts/provision-dev-vm.sh <gnome|kde> rdp      # IAP tunnel to localhost:13389
```

`create` sets `linux/scripts/linux-dev-bootstrap.sh` as the startup script.
It installs the desktop, GTK 4, libadwaita, Weston, Tesseract and rustup for
`tc`, turns on autologin, and reboots once into the desktop. A box is ready
when `sudo test -f /var/lib/tc-bootstrap.rebooted` succeeds and
`systemctl is-active display-manager` prints `active`;
`/var/log/tc-bootstrap.log` ends with `tc bootstrap done`. The RDP password
for `tc` is in `/root/tc-rdp-password` on the box.

## Building and running the shell

On a box, builds share four vCPUs, so take the lock around every cargo
command, and use the same flags as everywhere else:

```bash
git clone https://github.com/TraceCommons/trace-commons ~/work/<lane>
cd ~/work/<lane> && git fetch origin <branch> && git checkout <branch>
cd crates/trace-commons-contributor-gtk
RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0 flock /tmp/tc-cargo.lock cargo build
```

To run it inside `tc`'s session on recorded sample data (debug builds only;
see `macos/SAMPLE_DATA.md`):

```bash
sudo -u tc env XDG_RUNTIME_DIR=/run/user/$(id -u tc) WAYLAND_DISPLAY=wayland-0 \
  DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$(id -u tc)/bus \
  TRACE_COMMONS_SAMPLE=normalDay <path>/target/debug/trace-commons-shell
```

On sample data the shell opens the contribution window on the recorded set,
with a throwaway state directory, and never reaches a daemon.

## Screenshots

Take them on the box and copy them back with
`gcloud compute scp --tunnel-through-iap`:

- KDE: `spectacle -b -n -f -o <file>`.
- GNOME: `gnome-screenshot`, or run the app under a nested headless Weston
  and use `weston-screenshooter`, as `scripts/weston-portal-verify-inner.sh`
  does.

Name a window capture `<set>-<tab>-<panes>-<appearance>.png` and pair it
with the macOS capture using `scripts/parity/gallery.sh`.
