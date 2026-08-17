# The Windows dev VM

How to build, run, and see the WinUI contributor app on a GCE Windows instance,
driven from a machine that is not running Windows.

## Why this exists

The WinUI project cannot be built anywhere but Windows — the Windows App SDK
needs Visual Studio's MSBuild tooling — and it cannot be *seen* anywhere but a
real Windows desktop. CI proves it compiles. It cannot tell you the window
renders, the layout holds, or the daemon state actually reaches the header.

The interop layer is deliberately exempt from all of this: it targets plain
`net8.0` and its tests run against a macOS `.dylib`. Use the VM for the WinUI
half only.

## Cost, and stopping

An `e2-standard-4` Windows instance carries a per-vCPU Windows Server licence
charge on top of compute. It is roughly **$0.35–0.40/hour, about $9/day** if
left running. **Stop it when you are done:**

```bash
windows/scripts/provision-dev-vm.sh stop
```

`stop` re-reads the instance state from the API and prints it, rather than
reporting success from an exit code. Trust that output, not a memory of having
run it. `TERMINATED` means compute billing has ended; the boot disk still
bills, at roughly $10/month for 100 GB.

The Cloud Router and Cloud NAT the instance needs for egress **bill hourly on
their own**, whether or not the VM is running. If you are done for more than a
few days, delete them too:

```bash
gcloud compute routers delete tc-win-dev-router --region us-central1 --project tracecommons-pilot-2026
```

`provision-dev-vm.sh create` recreates both, so nothing is lost.

## The security model

The instance has **no external IP** (`--no-address`), and that is not
incidental. This project's `default-allow-rdp` firewall rule permits tcp:3389
from `0.0.0.0/0` with **no target tags**, so it applies to every instance in
the network. Firewall rules are additive-allow: a narrower rule cannot cancel
it. The only reliable defence is having no external address to reach.

Access therefore runs through IAP TCP forwarding, which Google IAM
authenticates before a packet reaches the host. Cloud NAT supplies outbound
internet without supplying inbound. SSH is public-key only; password
authentication is disabled in `sshd_config`.

## Setting one up

```bash
windows/scripts/provision-dev-vm.sh create     # instance, firewall, router, NAT
windows/scripts/provision-dev-vm.sh status     # authoritative state, via the API
```

`create` runs `win-dev-bootstrap.ps1` as the startup script, installing Visual
Studio Build Tools, the .NET 8 SDK, Rust, and Git. It takes roughly ten
minutes. Watch it:

```bash
gcloud compute instances get-serial-port-output tc-win-dev \
  --project tracecommons-pilot-2026 --zone us-central1-a \
  | grep -o '\[tc-bootstrap[^]]*\] .*' | tail
```

The bootstrap is idempotent behind a marker file, because GCE re-runs startup
scripts on every boot.

Then create the login account, which also produces the RDP credential:

```bash
gcloud compute reset-windows-password tc-win-dev \
  --project tracecommons-pilot-2026 --zone us-central1-a --user tcdev
```

## Working headlessly

`win-exec.sh` runs a PowerShell command on the box and prints the output
locally. It opens an IAP tunnel per invocation and tears it down after.

```bash
windows/scripts/win-exec.sh 'cargo --version'
windows/scripts/win-exec.sh 'cd C:\src\trace-commons-server; git fetch origin; git reset --hard FETCH_HEAD'
```

Build the cdylib first, then the app. Note **MSBuild, not `dotnet build`** —
see the gotchas.

```bash
windows/scripts/win-exec.sh '$env:Path += ";C:\Rust\cargo\bin"; cd C:\src\trace-commons-server; cargo build -p trace-commons-contributor-ffi --release'

windows/scripts/win-exec.sh 'cd C:\src\trace-commons-server\windows; $env:TC_FFI_LIB_DIR="C:\src\trace-commons-server\target\release"; & "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\MSBuild\Current\Bin\amd64\MSBuild.exe" src\TraceCommons.App\TraceCommons.App.csproj -restore -p:Configuration=Release -p:Platform=x64'
```

## Seeing the app

A GUI app cannot start from an SSH session — see the gotchas — so
`win-capture.ps1` hands the launch to a scheduled task in the interactive
console session and photographs the screen from there.

```bash
windows/scripts/win-exec.sh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\win-capture.ps1 -Exe "C:\src\...\TraceCommons.exe" -SettleSeconds 15'

# Pull the PNG back.
windows/scripts/win-exec.sh '[Convert]::ToBase64String([IO.File]::ReadAllBytes("C:\tc-captures\tc-capture.png"))' \
  | tail -1 | tr -d '\r\n ' | base64 -d > /tmp/screen.png
```

This requires a logged-on interactive session. Auto-logon provides one that
survives reboots:

```bash
# Sysinternals Autologon stores the credential as an encrypted LSA secret
# rather than plaintext in the registry.
windows/scripts/win-exec.sh 'C:\bootstrap-downloads\autologon\Autologon64.exe tcdev tc-win-dev <password> /accepteula'
```

The GCE guest agent's account manager **clears auto-logon settings on boot**,
so it must be disabled first, once:

```bash
gcloud compute instances add-metadata tc-win-dev \
  --project tracecommons-pilot-2026 --zone us-central1-a \
  --metadata disable-account-manager=true
```

## RDP, when you want to drive it yourself

```bash
windows/scripts/provision-dev-vm.sh rdp     # tunnels 3389 -> localhost:13389
```

Connect a client to `localhost:13389`. Prefer the console session for
automation: a **disconnected RDP session generally has no rendering surface**,
and screen captures from it come back black. The GCE console session has a
virtual display and works with nobody connected.

## Gotchas

Every one of these cost real debugging time, and each presents as something
other than what it is.

**A GUI app cannot start in session 0.** SSH lands in session 0, which has no
window station or desktop. A WinUI app launched there dies inside
`Microsoft.UI.Xaml.dll` with a stowed exception (`0xc000027b`) before any
managed application code runs. This looks exactly like an application bug.
Launch through a scheduled task bound to the interactive user instead.

**`dotnet build` cannot build a WinUI project.** It invokes MSBuild tasks from
`Microsoft.Build.AppxPackage.dll` and `Microsoft.Build.Packaging.Pri.Tasks.dll`,
which ship with Visual Studio and never with the .NET SDK. The resulting
`MSB4062` names a path under the *dotnet* SDK directory, so it reads like a
.NET version problem. Suppressing the first failing target with
`AppxGeneratePriEnabled=false` only surfaces the next task from the same
missing assembly set. Use MSBuild from Build Tools, with the
`UniversalBuildTools` and `ManagedDesktopBuildTools` workloads installed.

**MSBuild picks the newest installed SDK**, regardless of what `setup-dotnet`
installed. `windows/global.json` pins the major version; without it a machine
with .NET 10 present fails even though .NET 8 is installed.

**`Copy-Item` preserves the source timestamp.** Restoring a file from a backup
can leave it *older* than the build outputs, so MSBuild skips recompiling and
you keep testing the previous binary. This is the one that costs the most,
because it makes a fix look like it did not work. Force a rebuild with
`-t:Rebuild` whenever a revert appears to have no effect.

**`[IO.File]::WriteAllBytes` with a relative path ignores PowerShell's `cd`.**
.NET resolves relative paths against the *process* working directory, which
PowerShell's `Set-Location` does not change. Files silently land somewhere
else. Always pass absolute paths.

**Scheduled tasks created with `/IT` run unelevated**, so they cannot write to
`C:\`. The task still reports exit code 0, so the only symptom is a missing
file.

**PowerShell treats native stderr as a terminating error** when
`$ErrorActionPreference = 'Stop'`. `git clone` and `schtasks /Delete` both
write to stderr in entirely ordinary situations, and both will abort a script.

**Quotes inside a PowerShell here-string need no escaping.** Doubling them
writes them literally. Transfer file content as base64 and decode on the far
side rather than fighting the quoting.

**Startup scripts run as SYSTEM.** A per-user install lands in the SYSTEM
profile, where no interactive login can reach it — the box looks provisioned
and then reports the tool as missing. The bootstrap installs Rust to `C:\Rust`
with machine-wide `CARGO_HOME` and `RUSTUP_HOME` for this reason.

**The console session is 640×480** by default, which is tight for judging
layout. Raise it before doing serious visual work.

**The network discovery prompt** covers a third of that small screen on first
boot. The bootstrap disables it via
`HKLM:\SYSTEM\CurrentControlSet\Control\Network\NewNetworkWindowOff`.

**Raise the target window before capturing**, or the screenshot shows whatever
shell launched the task and a healthy app looks like one that never opened.
`win-capture.ps1` does this via `-ProcessName`.
