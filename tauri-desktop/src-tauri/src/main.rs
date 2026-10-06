mod app;
mod commands;
mod credential_store_check;
mod ipc;
#[cfg(test)]
mod macos_signing;
mod native;
mod runtime;
mod state;
mod tray;

fn main() {
    if credential_store_check::run_if_requested() {
        return;
    }
    app::run();
}
