//! Explicit local development harness; release builds refuse local configuration.
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use trace_commons_contributor::compute::{
    ComputeCommand, ComputeController, ComputeState, LocalWorkerConfig,
};

fn main() -> anyhow::Result<()> {
    // Inputs can contain local paths; never print argument or underlying error text.
    if run().is_err() {
        eprintln!("local-compute-validation-failed");
        std::process::exit(1);
    }
    Ok(())
}
fn run() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!((5..=6).contains(&args.len()), "arguments-required");
    let config = LocalWorkerConfig {
        binary: PathBuf::from(&args[1]),
        expected_sha256: args[2].clone(),
        coordinator: args[3].clone(),
        startup_timeout_secs: 30,
    };
    let allowance: u64 = args[4].parse()?;
    let observe: u64 = args.get(5).map(|s| s.parse()).transpose()?.unwrap_or(45);
    anyhow::ensure!((1..=300).contains(&observe), "observe-duration-invalid");
    let controller = ComputeController::open_local(std::path::Path::new(&args[0]), config)?;
    println!(
        "{}",
        serde_json::to_string(&controller.command(ComputeCommand::Enable {
            ram_allowance_gib: allowance
        }))?
    );
    let deadline = Instant::now() + Duration::from_secs(observe);
    let mut observed = false;
    let mut failed = false;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
        let snapshot = controller.snapshot();
        observed |= snapshot.admission.is_some();
        failed |= matches!(
            snapshot.state,
            ComputeState::Error | ComputeState::Unavailable
        );
        println!("{}", serde_json::to_string(&snapshot)?);
        if failed {
            break;
        }
    }
    let stopped = controller.shutdown(Duration::from_secs(30));
    println!("{}", serde_json::to_string(&stopped)?);
    anyhow::ensure!(
        !failed && observed && stopped.worker_stopped,
        "local-validation-failed"
    );
    Ok(())
}
