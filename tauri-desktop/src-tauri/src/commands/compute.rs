use tauri::State;
use trace_commons_contributor::compute::ComputeCommand;

use crate::state::{
    AppState, compute_command as state_compute_command, compute_status as state_compute_status,
};

#[tauri::command]
pub(crate) fn compute_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    state_compute_status(&state)
}

fn run_compute_command(
    state: &State<'_, AppState>,
    command: ComputeCommand,
) -> Result<serde_json::Value, String> {
    state_compute_command(state, command)
}

#[tauri::command]
pub(crate) fn enable_compute(
    state: State<'_, AppState>,
    ram_allowance_gib: u64,
) -> Result<serde_json::Value, String> {
    run_compute_command(&state, ComputeCommand::Enable { ram_allowance_gib })
}

#[tauri::command]
pub(crate) fn resume_compute(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    run_compute_command(&state, ComputeCommand::Resume {})
}

#[tauri::command]
pub(crate) fn pause_compute(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    run_compute_command(&state, ComputeCommand::Pause {})
}

#[tauri::command]
pub(crate) fn disable_compute(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    run_compute_command(&state, ComputeCommand::Disable {})
}
