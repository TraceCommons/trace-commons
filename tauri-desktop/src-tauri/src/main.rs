mod app;
mod commands;
mod ipc;
#[cfg(test)]
mod macos_signing;
mod native;
mod runtime;
mod state;
mod tray;

fn main() {
    app::run();
}
