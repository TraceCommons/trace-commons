# The Windows dev VM

How to build, run, and see the WinUI contributor app on a GCE Windows instance,
driven from a machine that is not running Windows.

## Why this exists

The WinUI project cannot be built anywhere but Windows — the Windows App SDK
needs Visual Studio's MSBuild tooling — and it cannot be *seen* anywhere but a
real Windows desktop. CI proves it compiles. It cannot tell you the window
renders, the layout holds, or the daemon state actually reaches the header.

There is also an Azure box, on Windows Server 2025, with a route to a GPU.
See "Azure: a Windows Server 2025 box" at the end of this page.

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

`provision-dev-vm.sh create` recreates both, so the *resources* are not
lost. The instance's internet is, until you run it — and the one-command
fix for that is not an external IP. Read the next two sections before
reaching for one.

### Deleting the NAT takes the instance's internet with it

That is the whole point of it, and it is easy to forget between sessions. The
symptom arrives later, as `git fetch` and `cargo` hanging on a box that
otherwise looks fine: IAP still gets you *in*, because that is Google's
infrastructure rather than the instance's own routing, so SSH works and only
egress is dead.

**The tempting fix is the wrong one.** Attaching an external IP restores
egress in one command, and it silently trades away the only thing standing
between this box and the internet — see the next section for why. It has
happened at least once: an access config was added on 2026-08-17 and removed
the following day, and for the ~37 minutes the instance was running in between,
tcp:3389 and tcp:22 were reachable from `0.0.0.0/0`.

Recreate the NAT instead:

```bash
windows/scripts/provision-dev-vm.sh create    # idempotent; recreates router + NAT
```

Check before trusting either, since neither is visible from inside the box:

```bash
# Should print nothing at all. Anything here is an external IP.
gcloud compute instances describe tc-win-dev --project tracecommons-pilot-2026 \
  --zone us-central1-a --format='get(networkInterfaces[0].accessConfigs)'

# Should exist, and hold a NAT.
gcloud compute routers describe tc-win-dev-router --region us-central1 \
  --project tracecommons-pilot-2026 --format='yaml(nats)'
```

## The security model

The instance has **no external IP** (`--no-address`), and that is not
incidental. This project carries two firewall rules,
`default-allow-rdp` (tcp:3389) and `default-allow-ssh` (tcp:22), each
permitting `0.0.0.0/0` with **no target tags**, so both apply to every instance
in the network. Firewall rules are additive-allow: a
narrower rule cannot cancel them, and the tagged `allow-iap-rdp` /
`allow-iap-ssh-win` rules alongside them narrow nothing. The only reliable
defence is having no external address to reach.

So the no-address property is load-bearing rather than tidy, and it is not
self-healing: nothing in `start` re-checks it, and an access config added once
persists across stop/start until someone deletes it. Verify it rather than
assume it — the command is in the section above.

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

## Azure: a Windows Server 2025 box

`provision-azure-vm.sh` builds a second test box, on Azure, for the same work.
It exists for two reasons. Azure offers GPU sizes (NVadsA10v5) that render
the WinUI app with hardware acceleration, and its Windows Server images
include the licence in the hourly price, so no separate Windows licence is
needed. The image is Windows Server 2025 Datacenter with Desktop Experience
(Gen2, Trusted Launch). The script never sets `--license-type`.

Defaults (each one can be changed with a `TC_AZ_*` variable at the top of the
script): resource group `tc-win-azure` in `eastus`, VM `tc-win-az`,
`Standard_D4s_v3` (4 vCPU, 16 GB), 128 GB Premium SSD, admin user `tcdev`.
Every resource is tagged `purpose=windows-parity-testing`.

```bash
windows/scripts/provision-azure-vm.sh create       # network, NSG, VM, sshd, auto-shutdown
windows/scripts/provision-azure-vm.sh bootstrap    # toolchain install, detached
TC_AZ_WAIT=540 windows/scripts/provision-azure-vm.sh bootstrap-log
```

`create` is idempotent: re-running it re-points the SSH rule, re-applies the
sshd lockdown, re-pins the host key and re-syncs the shutdown time. The
bootstrap is the same `win-dev-bootstrap.ps1` the GCE box runs at startup,
run here as a SYSTEM job. On a `D4s_v3` it took 25 minutes on 2026-11-18,
21 of them Visual Studio Build Tools.

Set `TC_AZ_EXPECTED_ACCOUNT` to the account you expect `az` to be signed in
as, and every command refuses to run under any other.

### Cost, and deallocating

Retail prices for `eastus` on 2026-11-18, from the Azure Retail Prices API:

| Item | Price |
|---|---|
| `Standard_D4s_v3`, Windows (licence included) | $0.376/hour |
| `Standard_NV6ads_A10_v5`, Windows (licence included) | $0.73/hour |
| 128 GB Premium SSD (P10) | $19.71/month |
| Standard static public IPv4 | $0.005/hour (about $3.65/month) |

**Deallocate it when you are done:**

```bash
windows/scripts/provision-azure-vm.sh stop
```

`stop` runs `az vm deallocate`, then re-reads the power state from the API
and exits non-zero unless it is `deallocated`. On Azure, "stopped" and
"deallocated" are different states. A VM shut down from inside Windows, or
with `az vm stop`, is *stopped*: it keeps its hardware and **still bills for
compute**. Only *deallocated* ends the compute and licence charge. The disk
and the public IP bill either way, about $23.36/month together. `delete`
removes the whole resource group.

An auto-shutdown schedule deallocates the VM every night at 01:00 local time,
as a safety net, not a substitute for `stop`. Azure takes the time in UTC, so
`create` converts 01:00 at today's UTC offset, and the time drifts an hour
across a daylight-saving change. Re-run `create` to re-sync it.

### Access model

The VM has a public IP. Its network security group admits exactly one inbound
flow: **tcp:22 from this machine's current public address, `/32`**. Azure's
default rules then deny everything else inbound. There is no 3389 rule and
never a `0.0.0.0/0` rule. `create` and `status` both list the NSG's rules and
fail if an inbound allow from a wildcard source, or one on 3389, has appeared.

- **The address is never written down.** The script resolves it at run time.
  When it changes (a new network, a new lease), SSH times out. Then run:

  ```bash
  windows/scripts/provision-azure-vm.sh allow-my-ip
  ```

  `status` says when the rule no longer matches your address.
- **SSH is by key only.** `create` installs a dedicated key,
  `~/.ssh/tc-win-az`, through `az vm run-command`, reusing
  `win-enable-ssh.ps1` (including the `administrators_authorized_keys` ACL).
  The sshd config sets `AuthenticationMethods publickey`, so the server offers
  no password or keyboard-interactive prompt at all; `create` checks this with
  `sshd -T` and refuses to finish otherwise.
- **The host key is pinned.** `create` reads the box's host key through
  run-command, which goes through the authenticated Azure control plane, and
  writes it to `~/.ssh/tc-win-az.known_hosts`. `win-exec-az.sh` connects with
  `StrictHostKeyChecking=yes`. That differs from `win-exec.sh`, which can
  skip host-key checks because IAP authenticates its tunnel. A public IP has
  no such layer.
- **The admin password** is generated by `create`, stored in the macOS
  Keychain (service `tc-win-az`, account `tcdev`) before the VM exists, and
  never printed. It is needed only for RDP and console auto-logon:

  ```bash
  security find-generic-password -s tc-win-az -a tcdev -w
  ```

### Why there is no public RDP

An internet-facing 3389 is scanned and credential-stuffed within minutes of
appearing, and RDP has had pre-authentication vulnerabilities. RDP runs inside
the SSH connection instead:

```bash
windows/scripts/provision-azure-vm.sh rdp     # forwards localhost:13389 to the box's 3389
```

Connect an RDP client to `localhost:13389` as `tcdev`.

**Do not let `az vm create` build the network.** Given no NIC, `az vm create`
makes its own NSG and, for a Windows image, adds a rule opening 3389 to any
source. The script creates the NSG, public IP and NIC first and passes
`--nics`, so that rule never exists.

### Working headlessly

`win-exec-az.sh` is the counterpart of `win-exec.sh`:

```bash
windows/scripts/win-exec-az.sh 'cargo --version'
```

The command travels as `-EncodedCommand`, so it needs no escaping. Standard
input is passed through, which is how a secret reaches the box without
appearing in a command line or a stored script.

**Long work runs as a job.** A process started from an SSH session dies when
the session closes, so a cold build cannot ride on one connection. `--job`
runs the command as a detached scheduled task that writes a log and an
exit-code file. `--wait` waits on the box and prints the log's tail:

```bash
windows/scripts/win-exec-az.sh --job ffi 'Set-Location C:\src\trace-commons; cargo build -p trace-commons-contributor-ffi'
windows/scripts/win-exec-az.sh --wait ffi 540   # exit: the job's code, or 75 while still running
```

A job runs as `tcdev` through an S4U logon, with no stored password.
`--system` runs it as SYSTEM, which the bootstrap needs.

### Seeing the app

The steps are the same as for the GCE box (see "Seeing the app" above), and
need the same console auto-logon. Install it once, piping the password from
the Keychain over SSH:

```bash
security find-generic-password -s tc-win-az -a tcdev -w | windows/scripts/win-exec-az.sh \
  '$p = [Console]::In.ReadLine(); $r = Start-Process -FilePath C:\bootstrap-downloads\autologon\Autologon64.exe -ArgumentList @("tcdev", $env:COMPUTERNAME, $p, "/accepteula") -Wait -PassThru; $p = $null; "Autologon exit: $($r.ExitCode)"'
windows/scripts/win-exec-az.sh 'Restart-Computer -Force'
```

The Azure guest agent does not clear auto-logon (checked across two
restarts on 2026-11-18), so no step like GCE's `disable-account-manager` is
needed. Autologon stores the password as an LSA secret: the `DefaultPassword`
registry value stays empty.

`bootstrap` copies `win-capture.ps1` to `C:\tc`, so:

```bash
windows/scripts/win-exec-az.sh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\tc\win-capture.ps1 -Exe "C:\src\trace-commons\windows\src\TraceCommons.App\bin\x64\Release\net8.0-windows10.0.19041.0\win-x64\TraceCommons.exe" -SettleSeconds 25'
scp -i ~/.ssh/tc-win-az -o HostKeyAlias=tc-win-az -o UserKnownHostsFile=~/.ssh/tc-win-az.known_hosts \
  "tcdev@$(az network public-ip show -g tc-win-azure -n tc-win-az-ip --query ipAddress -o tsv):C:/tc-captures/tc-capture.png" /tmp/screen.png
```

The Azure console session is 1024x768, not GCE's 640x480.

**Windows Server 2025 opens a full-screen "Send diagnostic data to
Microsoft" page at first logon**, above every other window, so the first
capture shows that page and not the app. The bootstrap sets the
`DisablePrivacyExperience` policy to skip it. The policy acts at logon, so on
a box whose first logon happened before the policy was set, restart once.

### Moving to a GPU

The GPU size is `Standard_NV6ads_A10_v5`: 6 vCPU and a sixth of an A10.
`resize` deallocates the VM, changes the size and starts it again:

```bash
windows/scripts/provision-azure-vm.sh resize Standard_NV6ads_A10_v5
```

That needs `StandardNVADSA10v5Family` vCPU quota in the region. A new
pay-as-you-go subscription has none, and the regional `cores` total (10 by
default) also has to cover it. Not yet tried here: the GPU drivers an A10
size needs on Windows, and a resize to it.

**`az quota create` did not raise this family on this pay-as-you-go
subscription.** On 2026-11-18 it failed in `eastus`, `westus2` and
`southcentralus` with `QuotaNotAvailableForResource`. The remaining route is
a quota request through Azure support: in the portal, Help + support, then
"Service and subscription limits (quotas)".
