#!/usr/bin/env bash
#
# Provision a Windows Server test box on Azure for building and RUNNING the
# WinUI contributor app. The Azure counterpart of provision-dev-vm.sh (GCE).
#
# Why a second box: Azure has GPU sizes (NVadsA10v5) that render the WinUI
# app with real hardware acceleration, and its Windows Server images carry the
# licence in the hourly price, so no separate Windows licence is needed. The
# image is Windows Server 2025 with Desktop Experience. This script never sets
# --license-type: that would claim a licence we do not hold.
#
# ACCESS IS SSH BY KEY, FROM ONE ADDRESS. The VM has a public IP, but its
# network security group admits exactly one inbound flow: tcp:22 from this
# machine's current public address, /32. There is no 3389 rule and never a
# 0.0.0.0/0 rule. RDP, when wanted, runs inside the SSH connection
# (`rdp` below). An internet-facing RDP port is a standing credential-stuffing
# target. `create` and `status` both check the NSG and fail loudly if any
# wider inbound allow has appeared.
#
# The address is resolved when the script runs, never written into the repo.
# When your address changes, run `allow-my-ip`.
#
# The admin password is generated here, stored in the macOS Keychain
# (service = the VM's name, account tcdev), and never printed. sshd on the box
# refuses passwords; the password is only for RDP and console auto-logon.
#
# COST. Compute and the Windows licence bill while the VM is running. `stop`
# DEALLOCATES and then re-reads the power state from the API: a VM that is
# merely "stopped" (shut down from inside Windows) still bills for compute.
# The OS disk and the static public IP bill even while deallocated.
#
#   ./provision-azure-vm.sh stop
#   ./provision-azure-vm.sh status
#
# SEVERAL BOXES. TC_AZ_VM picks the box (default tc-win-az); every per-box
# name derives from it: the NIC, NSG, public IP, VNet, OS disk, the pinned
# known_hosts file (~/.ssh/<vm>.known_hosts) and the Keychain service. All
# boxes share one resource group and one SSH key. TC_AZ_LOCATION picks the
# region a NEW box is created in; the group's own region does not limit it.
#
#   TC_AZ_VM=tc-win-az3 TC_AZ_LOCATION=westus2 ./provision-azure-vm.sh create
#   ./provision-azure-vm.sh list      # every box tagged purpose=windows-parity-testing
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

RG="${TC_AZ_RG:-tc-win-azure}"
LOCATION="${TC_AZ_LOCATION:-eastus}"
NAME="${TC_AZ_VM:-tc-win-az}"
SIZE="${TC_AZ_SIZE:-Standard_D4s_v3}"
DISK_GB="${TC_AZ_DISK_GB:-128}"
ADMIN_USER="${TC_AZ_USER:-tcdev}"
IMAGE="${TC_AZ_IMAGE:-MicrosoftWindowsServer:WindowsServer:2025-datacenter-g2:latest}"
KEY="${TC_AZ_KEY:-$HOME/.ssh/tc-win-az}"
KNOWN_HOSTS="${TC_AZ_KNOWN_HOSTS:-$HOME/.ssh/${NAME}.known_hosts}"
# Optional guard: set to the account you expect `az` to be signed in as, and
# every command refuses to run under any other.
EXPECTED_ACCOUNT="${TC_AZ_EXPECTED_ACCOUNT:-}"
# Local wall-clock time for the nightly auto-shutdown safety net.
SHUTDOWN_LOCAL="${TC_AZ_SHUTDOWN_LOCAL:-0100}"

KEYCHAIN_SERVICE="${TC_AZ_KEYCHAIN_SERVICE:-$NAME}"
VNET="${NAME}-vnet"
SUBNET="default"
NSG="${NAME}-nsg"
NSG_RULE="ssh-from-operator"
PIP="${NAME}-ip"
NIC="${NAME}-nic"
TAG_KEY="purpose"
TAG_VALUE="windows-parity-testing"
TAG="${TAG_KEY}=${TAG_VALUE}"
# az vm auto-shutdown names its schedule after the VM; looked up by that name
# so one box's schedule is never mistaken for another's.
SCHEDULE="shutdown-computevm-${NAME}"
# HostKeyAlias, so the pinned key is tied to the box rather than to an IP.
HOST_ALIAS="$NAME"

usage() {
  cat <<'USAGE'
Usage: [TC_AZ_VM=<vm>] [TC_AZ_LOCATION=<region>] provision-azure-vm.sh <command>

  create        Create the VM (network, NSG, VM, SSH, auto-shutdown)
  bootstrap     Start the toolchain install on the box (detached; poll it)
  bootstrap-log Show the bootstrap's progress and, once done, its exit code
  status        Show power state, size, IP and the NSG rule (via the API)
  start         Start the VM
  stop          DEALLOCATE the VM, then re-read and print the power state
  ssh           Open an interactive SSH session
  rdp           Tunnel the box's 3389 to localhost:13389 over SSH
  allow-my-ip   Point the NSG's SSH rule at this machine's current address
  resize SIZE   Deallocate, change the VM size, and start it again
  list          Every VM tagged purpose=windows-parity-testing, in any group
                or region, with its size and power state (via the API)
  delete        Delete THIS VM and its own disk, NIC, public IP, NSG, VNet and
                shutdown schedule; never the group or another box

TC_AZ_VM selects the box (default tc-win-az). TC_AZ_LOCATION sets the region
for create (default eastus).
USAGE
}

az_q() { az "$@" --only-show-errors; }

my_ip() {
  local ip
  ip="$(curl -fsS --max-time 10 https://api.ipify.org)"
  if ! [[ "$ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "could not determine this machine's public IPv4 address (got '$ip')" >&2
    exit 1
  fi
  printf '%s' "$ip"
}

# Preflight, every time. A wrong or expired account should stop us here rather
# than halfway through creating billable resources in someone else's
# subscription.
preflight() {
  command -v az >/dev/null 2>&1 || { echo "az is not installed" >&2; exit 1; }
  local account sub
  if ! account="$(az account show --query user.name -o tsv 2>/dev/null)"; then
    echo "az is not signed in. Run: az login" >&2
    exit 1
  fi
  sub="$(az account show --query name -o tsv)"
  if [ -n "$EXPECTED_ACCOUNT" ] && \
     [ "$(printf '%s' "$account" | tr '[:upper:]' '[:lower:]')" != \
       "$(printf '%s' "$EXPECTED_ACCOUNT" | tr '[:upper:]' '[:lower:]')" ]; then
    echo "az is signed in as $account, expected $EXPECTED_ACCOUNT. Refusing." >&2
    exit 1
  fi
  echo "account:      $account"
  echo "subscription: $sub"
  echo "group:        $RG"
  echo "vm:           $NAME"
}

public_ip() {
  az_q network public-ip show -g "$RG" -n "$PIP" --query ipAddress -o tsv
}

# ---------------------------------------------------------------------------
# The admin password.
#
# Generated and stored BEFORE the VM exists, so a failed Keychain write can
# never leave a VM behind whose password nobody knows. A re-run reuses the
# stored one. The alphabet avoids quotes, `$`, backtick and backslash so the
# value survives shell and PowerShell quoting untouched, and the loop insists
# on all four character classes Azure's complexity rule counts.
# ---------------------------------------------------------------------------
keychain_password() {
  security find-generic-password -s "$KEYCHAIN_SERVICE" -a "$ADMIN_USER" -w 2>/dev/null
}

ensure_password() {
  if keychain_password >/dev/null; then
    echo "admin password: already in the Keychain (service $KEYCHAIN_SERVICE, account $ADMIN_USER)"
    return
  fi
  local pw=""
  while :; do
    # Bounded input rather than `tr </dev/urandom | head`: under pipefail the
    # SIGPIPE that head hands tr fails the whole script.
    pw="$(head -c 1024 /dev/urandom | LC_ALL=C tr -dc 'A-Za-z0-9_.:+=%-')"
    pw="${pw:0:28}"
    [ "${#pw}" -eq 28 ] || continue
    [[ "$pw" =~ [A-Z] && "$pw" =~ [a-z] && "$pw" =~ [0-9] && "$pw" =~ [_.:+=%-] ]] && break
  done
  security add-generic-password -s "$KEYCHAIN_SERVICE" -a "$ADMIN_USER" \
    -l "Azure Windows test box ($NAME)" -w "$pw"
  echo "admin password: generated and stored in the Keychain (service $KEYCHAIN_SERVICE, account $ADMIN_USER)"
}

ensure_key() {
  if [ ! -f "$KEY" ]; then
    echo "creating SSH key $KEY"
    ssh-keygen -t ed25519 -N '' -C "$NAME headless build access" -f "$KEY" -q
  fi
}

# ---------------------------------------------------------------------------
# The NSG check. Run after create and on every status.
#
# Azure's default rules end in DenyAllInBound, so the custom rules are the
# whole of what is reachable. Any inbound Allow from a wildcard source, or any
# rule mentioning 3389, is a defect to fix now, not later.
# ---------------------------------------------------------------------------
check_nsg() {
  local bad
  bad="$(az_q network nsg rule list -g "$RG" --nsg-name "$NSG" -o tsv \
    --query "[?direction=='Inbound' && access=='Allow' && (sourceAddressPrefix=='*' || sourceAddressPrefix=='Internet' || sourceAddressPrefix=='0.0.0.0/0' || contains(join(',', sourceAddressPrefixes), '0.0.0.0/0') || destinationPortRange=='*' || destinationPortRange=='3389' || contains(join(',', destinationPortRanges), '3389'))].name")"
  if [ -n "$bad" ]; then
    echo "NSG $NSG HAS A WIDE OR RDP INBOUND RULE: $bad" >&2
    echo "Delete it: az network nsg rule delete -g $RG --nsg-name $NSG -n <rule>" >&2
    return 1
  fi
  local count
  count="$(az_q network nsg rule list -g "$RG" --nsg-name "$NSG" \
    --query "length([?direction=='Inbound'])" -o tsv)"
  echo "NSG check: $count custom inbound rule(s), none wide, none on 3389"
}

ensure_network() {
  local ip="$1"
  # The group holds several boxes in several regions. Re-running `group
  # create` against an existing group with a different -l is an error, so
  # create it only when it is missing.
  if [ "$(az_q group exists -n "$RG")" != "true" ]; then
    az_q group create -n "$RG" -l "$LOCATION" --tags "$TAG" -o none
  fi
  echo "resource group $RG ready; this box goes in $LOCATION"

  if ! az_q network nsg show -g "$RG" -n "$NSG" -o none 2>/dev/null; then
    az_q network nsg create -g "$RG" -n "$NSG" -l "$LOCATION" --tags "$TAG" -o none
    echo "created NSG $NSG"
  fi
  # The one inbound rule. Created or re-pointed, never widened.
  az_q network nsg rule create -g "$RG" --nsg-name "$NSG" -n "$NSG_RULE" \
    --priority 100 --direction Inbound --access Allow --protocol Tcp \
    --source-address-prefixes "${ip}/32" --source-port-ranges '*' \
    --destination-address-prefixes '*' --destination-port-ranges 22 \
    --description "SSH from the operator's current address only" -o none
  echo "NSG rule $NSG_RULE: tcp/22 from this machine's address /32"

  if ! az_q network vnet show -g "$RG" -n "$VNET" -o none 2>/dev/null; then
    az_q network vnet create -g "$RG" -n "$VNET" -l "$LOCATION" \
      --address-prefixes 10.42.0.0/16 --subnet-name "$SUBNET" \
      --subnet-prefixes 10.42.0.0/24 --tags "$TAG" -o none
    echo "created VNet $VNET"
  fi

  if ! az_q network public-ip show -g "$RG" -n "$PIP" -o none 2>/dev/null; then
    # Standard SKU is closed by default: nothing reaches the NIC unless an
    # NSG rule allows it.
    az_q network public-ip create -g "$RG" -n "$PIP" -l "$LOCATION" \
      --sku Standard --allocation-method Static --version IPv4 --tags "$TAG" -o none
    echo "created public IP $PIP"
  fi

  if ! az_q network nic show -g "$RG" -n "$NIC" -o none 2>/dev/null; then
    az_q network nic create -g "$RG" -n "$NIC" -l "$LOCATION" \
      --vnet-name "$VNET" --subnet "$SUBNET" \
      --network-security-group "$NSG" --public-ip-address "$PIP" \
      --tags "$TAG" -o none
    echo "created NIC $NIC (NSG $NSG attached)"
  fi
}

# Install and lock down sshd through the Azure guest agent.
#
# Reuses win-enable-ssh.ps1 -- the administrators_authorized_keys ACL and the
# keys-only sshd_config are exactly what is needed here -- but swaps in this
# box's own public key, read from the local .pub at run time. The script then
# prints the host key, which we pin, and a sentinel line we require, because
# run-command can report success for a script that threw.
enable_ssh() {
  local pub tmp out
  pub="$(cat "${KEY}.pub")"
  tmp="$(mktemp -t tc-win-az-ssh).ps1"
  local count
  count="$(grep -c "^\$authorizedKey = '" "$SCRIPT_DIR/win-enable-ssh.ps1" || true)"
  if [ "$count" != "1" ]; then
    echo "win-enable-ssh.ps1 no longer has exactly one \$authorizedKey line; update enable_ssh()" >&2
    exit 1
  fi
  sed "s|^\$authorizedKey = '.*'\$|\$authorizedKey = '${pub}'|" \
    "$SCRIPT_DIR/win-enable-ssh.ps1" >"$tmp"
  cat >>"$tmp" <<'PS'

# --- Azure additions --------------------------------------------------------
# Prove the effective config, rather than trusting the edit above.
$effective = & 'C:\Windows\System32\OpenSSH\sshd.exe' -T 2>&1 |
    Select-String -Pattern '^(passwordauthentication|pubkeyauthentication|kbdinteractiveauthentication|authenticationmethods) '
Write-Log "sshd -T: $($effective -join '; ')"
$fw = Get-NetFirewallRule -Name 'sshd-tc'
Write-Log "firewall rule sshd-tc: enabled=$($fw.Enabled) profile=$($fw.Profile)"
Write-Output ('TC_HOSTKEY ' + (Get-Content 'C:\ProgramData\ssh\ssh_host_ed25519_key.pub' -Raw).Trim())
Write-Output 'TC_SSH_READY'
PS

  echo "enabling OpenSSH on the box (run-command; a few minutes)"
  out="$(az_q vm run-command invoke -g "$RG" -n "$NAME" --command-id RunPowerShellScript \
    --scripts "@$tmp" --query 'value[].message' -o tsv)"
  rm -f "$tmp"
  printf '%s\n' "$out" | grep -E '^\[tc-ssh|^TC_' | grep -v '^TC_HOSTKEY' || true
  if ! printf '%s\n' "$out" | grep -q '^TC_SSH_READY'; then
    echo "SSH enablement did not complete. Full output:" >&2
    printf '%s\n' "$out" >&2
    exit 1
  fi
  if ! printf '%s\n' "$out" | grep -q 'passwordauthentication no' || \
     ! printf '%s\n' "$out" | grep -q 'authenticationmethods publickey'; then
    echo "sshd still offers a password method; refusing to continue" >&2
    exit 1
  fi
  local hostkey
  hostkey="$(printf '%s\n' "$out" | sed -n 's/^TC_HOSTKEY //p' | tr -d '\r')"
  if [ -z "$hostkey" ]; then
    echo "run-command did not return the host key" >&2
    exit 1
  fi
  # Pinned host key, fetched through the authenticated Azure control plane
  # rather than trusted on first use over the network.
  printf '%s %s\n' "$HOST_ALIAS" "$(printf '%s' "$hostkey" | awk '{print $1, $2}')" >"$KNOWN_HOSTS"
  chmod 600 "$KNOWN_HOSTS"
  echo "pinned host key in $KNOWN_HOSTS"
}

# 01:00 local, converted to the UTC hhmm that `az vm auto-shutdown` takes.
# Computed for today's offset, so it drifts by an hour across a DST change;
# re-run `create` (idempotent) to re-sync it if that matters.
shutdown_utc() {
  local today
  today="$(date +%Y-%m-%d)"
  date -u -r "$(date -j -f '%Y-%m-%d %H%M%S' "$today ${SHUTDOWN_LOCAL}00" +%s)" +%H%M
}

ensure_autoshutdown() {
  local utc
  utc="$(shutdown_utc)"
  # The schedule must sit in the VM's region, and az defaults it to the
  # group's. Read the VM's region from the API rather than TC_AZ_LOCATION, so
  # a re-run that leaves the variable unset still gets it right.
  local vm_loc
  vm_loc="$(az_q vm show -g "$RG" -n "$NAME" --query location -o tsv)"
  az_q vm auto-shutdown -g "$RG" -n "$NAME" -l "$vm_loc" --time "$utc" -o none
  local sched_id
  sched_id="$(az_q resource list -g "$RG" --resource-type Microsoft.DevTestLab/schedules \
    --query "[?name=='${SCHEDULE}'].id | [0]" -o tsv)"
  if [ -z "$sched_id" ]; then
    echo "auto-shutdown schedule $SCHEDULE not found after creating it" >&2
    exit 1
  fi
  az_q resource tag --ids "$sched_id" --tags "$TAG" -i -o none
  echo "auto-shutdown: daily at ${utc} UTC (${SHUTDOWN_LOCAL} local today)"
  az_q resource show --ids "$sched_id" \
    --query '{status:properties.status,time:properties.dailyRecurrence.time,tz:properties.timeZoneId}' -o tsv
}

# Every resource in the group carries the purpose tag, including the ones
# az vm create makes on its own (the OS disk).
tag_everything() {
  local id
  for id in $(az_q resource list -g "$RG" --query "[?tags.purpose!='windows-parity-testing'].id" -o tsv); do
    az_q resource tag --ids "$id" --tags "$TAG" -i -o none
  done
  az_q resource list -g "$RG" --query '[].{name:name,type:type,purpose:tags.purpose}' -o table
}

create() {
  preflight
  ensure_key
  ensure_password
  local ip
  ip="$(my_ip)"
  ensure_network "$ip"
  check_nsg

  if az_q vm show -g "$RG" -n "$NAME" -o none 2>/dev/null; then
    echo "VM $NAME already exists"
  else
    echo "creating $NAME ($SIZE, ${DISK_GB} GB Premium SSD, $IMAGE)"
    local pw
    pw="$(keychain_password)"
    # --nics with a pre-made NIC is load-bearing: given no NIC, az vm create
    # makes its own NSG and, for a Windows image, opens 3389 to the internet.
    # No zone: zonal capacity for this size is restricted on some
    # subscriptions, and a zonal VM gains nothing here.
    az_q vm create -g "$RG" -n "$NAME" -l "$LOCATION" \
      --image "$IMAGE" --size "$SIZE" \
      --nics "$NIC" \
      --admin-username "$ADMIN_USER" --admin-password "$pw" \
      --os-disk-size-gb "$DISK_GB" --storage-sku Premium_LRS \
      --os-disk-name "${NAME}-osdisk" --os-disk-delete-option Delete \
      --security-type TrustedLaunch --enable-secure-boot true --enable-vtpm true \
      --tags "$TAG" \
      --query '{id:id,powerState:powerState,publicIp:publicIpAddress}' -o table
    pw=""
  fi

  enable_ssh
  ensure_autoshutdown
  tag_everything
  check_nsg

  cat <<NEXT

Created. Next:
  $0 bootstrap        # install VS Build Tools, .NET 8, Rust, Git (detached)
  $0 bootstrap-log    # poll it
  $0 stop             # deallocate when done

NEXT
  status
}

ssh_base() {
  local ip
  ip="$(public_ip)"
  if [ -z "$ip" ]; then
    echo "VM $NAME has no public IP" >&2
    exit 1
  fi
  SSH_ARGS=(-i "$KEY"
    -o IdentitiesOnly=yes
    -o HostKeyAlias="$HOST_ALIAS"
    -o UserKnownHostsFile="$KNOWN_HOSTS"
    -o StrictHostKeyChecking=yes
    -o ConnectTimeout=15
    -o LogLevel=ERROR
    "${ADMIN_USER}@${ip}")
}

# The bootstrap is far longer than any one connection should be held open, and
# a child of the SSH session dies with the session. So it runs as a detached
# SYSTEM job (win-exec-az.sh --job) -- the same identity GCE's startup script
# uses, which the bootstrap's machine-wide installs assume -- writing a log and
# an exit-code file that bootstrap-log reads.
bootstrap() {
  preflight
  ssh_base
  echo 'copying win-dev-bootstrap.ps1 and win-capture.ps1 to C:\tc on the box'
  "$SCRIPT_DIR/win-exec-az.sh" 'New-Item -ItemType Directory -Force C:\tc | Out-Null'
  scp -q -i "$KEY" -o IdentitiesOnly=yes -o HostKeyAlias="$HOST_ALIAS" \
    -o UserKnownHostsFile="$KNOWN_HOSTS" -o StrictHostKeyChecking=yes \
    "$SCRIPT_DIR/win-dev-bootstrap.ps1" "$SCRIPT_DIR/win-capture.ps1" \
    "${ADMIN_USER}@$(public_ip):C:/tc/"
  "$SCRIPT_DIR/win-exec-az.sh" --job bootstrap --system '& C:\tc\win-dev-bootstrap.ps1'
}

bootstrap_log() {
  # Waits up to TC_AZ_WAIT seconds on the box (default: just look).
  exec "$SCRIPT_DIR/win-exec-az.sh" --wait bootstrap "${TC_AZ_WAIT:-0}"
}

status() {
  preflight
  # Authoritative: ask the API rather than trusting what anyone remembers.
  az_q vm get-instance-view -g "$RG" -n "$NAME" -o table --query \
    '{name:name, size:hardwareProfile.vmSize, power:instanceView.statuses[?starts_with(code, `PowerState/`)] | [0].displayStatus, location:location}'
  echo "public IP:    $(public_ip)"
  local src
  src="$(az_q network nsg rule show -g "$RG" --nsg-name "$NSG" -n "$NSG_RULE" \
    --query 'sourceAddressPrefix || join(`,`, sourceAddressPrefixes)' -o tsv)"
  echo "SSH allowed from: $src"
  local here
  here="$(my_ip)"
  if [ "$src" != "${here}/32" ]; then
    echo "  (not this machine's current address; run: $0 allow-my-ip)"
  fi
  check_nsg
}

power_state() {
  az_q vm get-instance-view -g "$RG" -n "$NAME" \
    --query 'instanceView.statuses[?starts_with(code, `PowerState/`)] | [0].code' -o tsv
}

start() {
  preflight
  az_q vm start -g "$RG" -n "$NAME" -o none
  echo
  echo "Verified state after start:"
  status
}

stop() {
  preflight
  # DEALLOCATE, not stop. `az vm stop` (or shutting down inside Windows)
  # leaves the VM allocated, and an allocated VM bills for compute.
  az_q vm deallocate -g "$RG" -n "$NAME" -o none
  # Re-read the state rather than reporting success from the exit code.
  echo
  echo "Verified state after stop:"
  status
  local state
  state="$(power_state)"
  if [ "$state" != "PowerState/deallocated" ]; then
    echo "VM IS NOT DEALLOCATED (power state: $state). It is still billing." >&2
    exit 1
  fi
}

ssh_cmd() {
  ssh_base
  exec ssh "${SSH_ARGS[@]}"
}

rdp() {
  ssh_base
  echo "Tunnelling the box's 3389 -> localhost:13389 over SSH."
  echo "Connect an RDP client to localhost:13389 as $ADMIN_USER. The password is in"
  echo "the Keychain: security find-generic-password -s $KEYCHAIN_SERVICE -a $ADMIN_USER -w"
  echo "Ctrl-C to close the tunnel."
  exec ssh -N -L 13389:localhost:3389 "${SSH_ARGS[@]}"
}

allow_my_ip() {
  preflight
  local ip
  ip="$(my_ip)"
  az_q network nsg rule update -g "$RG" --nsg-name "$NSG" -n "$NSG_RULE" \
    --source-address-prefixes "${ip}/32" -o none
  echo "SSH rule now admits only this machine's current address."
  check_nsg
}

# Read from the API, not from this machine's files: every VM in the
# subscription with the purpose tag, whichever group or region it is in.
# `az vm list -d` fetches each VM's instance view, so the power state is live.
list_vms() {
  preflight
  az_q vm list -d -o table --query \
    "[?tags.${TAG_KEY}=='${TAG_VALUE}'].{name:name, group:resourceGroup, region:location, size:hardwareProfile.vmSize, power:powerState, ip:publicIps}"
}

resize() {
  local size="${1:-}"
  if [ -z "$size" ]; then
    echo "usage: provision-azure-vm.sh resize <size>   e.g. Standard_NV6ads_A10_v5" >&2
    exit 1
  fi
  preflight
  # Deallocate first: a size outside the current hardware cluster (any change
  # of family, GPU sizes included) can only be applied to a deallocated VM.
  az_q vm deallocate -g "$RG" -n "$NAME" -o none
  az_q vm resize -g "$RG" -n "$NAME" --size "$size" -o none
  az_q vm start -g "$RG" -n "$NAME" -o none
  echo
  echo "Verified state after resize:"
  status
}

# Deletes this one box. The group holds every box, so deleting the group --
# which this command once did -- would take the others with it.
delete() {
  preflight
  if ! az_q vm show -g "$RG" -n "$NAME" -o none 2>/dev/null; then
    echo "no VM $NAME in $RG" >&2
    exit 1
  fi
  echo "Deleting VM $NAME and its own resources: ${NAME}-osdisk, $NIC, $PIP, $NSG, $VNET, $SCHEDULE."
  # The OS disk was created with --os-disk-delete-option Delete and goes with
  # the VM. The NIC was made first and passed with --nics, so it does not.
  az_q vm delete -g "$RG" -n "$NAME" --yes
  az_q disk delete -g "$RG" -n "${NAME}-osdisk" --yes 2>/dev/null || true
  az_q resource delete -g "$RG" -n "$SCHEDULE" \
    --resource-type Microsoft.DevTestLab/schedules 2>/dev/null || true
  az_q network nic delete -g "$RG" -n "$NIC"
  az_q network public-ip delete -g "$RG" -n "$PIP"
  az_q network nsg delete -g "$RG" -n "$NSG"
  az_q network vnet delete -g "$RG" -n "$VNET"
  echo "Deleted. The Keychain entry (service $KEYCHAIN_SERVICE) and $KNOWN_HOSTS remain;"
  echo "remove them if the box is not coming back."
}

case "${1:-}" in
  create) create ;;
  bootstrap) bootstrap ;;
  bootstrap-log) bootstrap_log ;;
  status) status ;;
  start) start ;;
  stop) stop ;;
  ssh) ssh_cmd ;;
  rdp) rdp ;;
  allow-my-ip) allow_my_ip ;;
  resize) shift; resize "${1:-}" ;;
  list) list_vms ;;
  delete) delete ;;
  *) usage; exit 1 ;;
esac
