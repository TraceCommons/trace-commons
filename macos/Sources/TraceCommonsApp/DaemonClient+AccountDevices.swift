import TCShellCore

// The device IPC calls (`account_devices_list`, `account_device_rename`,
// `account_device_revoke`, `account_device_code_mint`,
// `account_device_code_status`) go through `AccountDevicesClient` in
// TCShellCore, so their method names and parameters are asserted without a
// daemon. Calls block (the ABI is synchronous): callers run them off the
// main actor.
extension DaemonClient {
    var accountDevices: AccountDevicesClient { AccountDevicesClient(transport: daemon) }
}
