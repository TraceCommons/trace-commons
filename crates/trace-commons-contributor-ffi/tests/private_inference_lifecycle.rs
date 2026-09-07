//! Exercise the shipping synchronous C path on its persistent daemon runtime.

use std::ffi::{CStr, CString, c_char};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::daemon::settings::{DaemonSettings, SourceDeclaration};
use trace_commons_contributor_ffi::{
    tc_call, tc_daemon_start, tc_daemon_stop, tc_handle, tc_handle_free, tc_string_free,
};

struct Handle(*mut tc_handle);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            tc_daemon_stop(self.0);
            tc_handle_free(self.0);
        }
    }
}

fn call(handle: &Handle, method: &str, params: &str) -> serde_json::Value {
    let method = CString::new(method).unwrap();
    let params = CString::new(params).unwrap();
    let out = unsafe { tc_call(handle.0, method.as_ptr(), params.as_ptr()) };
    assert!(!out.is_null());
    let body = unsafe { CStr::from_ptr(out) }.to_bytes().to_vec();
    unsafe { tc_string_free(out) };
    let frame: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        frame.get("error").is_none_or(serde_json::Value::is_null),
        "{frame}"
    );
    frame["result"].clone()
}

fn health_answers(port: u16) -> bool {
    let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .write_all(
            b"GET /_ironwire/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    bytes.starts_with(b"HTTP/1.1 200")
}

fn exercise_child(root: &Path) {
    let state_dir = root.join("state");
    let store = ConfigStore::open(state_dir.clone()).unwrap();
    DaemonSettings {
        claude_source: Some(SourceDeclaration::Off),
        codex_source: Some(SourceDeclaration::Off),
        gemini_source: Some(SourceDeclaration::Off),
        cline_source: Some(SourceDeclaration::Off),
        ..Default::default()
    }
    .save(&store)
    .unwrap();
    let dir = CString::new(state_dir.to_str().unwrap()).unwrap();
    let mut error: *mut c_char = std::ptr::null_mut();
    let raw = unsafe { tc_daemon_start(dir.as_ptr(), &mut error) };
    if raw.is_null() {
        assert!(!error.is_null(), "failed start must supply its refusal");
        let message = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        unsafe { tc_string_free(error) };
        panic!("isolated daemon failed: {message}");
    }
    let handle = Handle(raw);
    assert_eq!(
        call(&handle, "get_settings", "{}")["private_inference_state"]["state"],
        "off"
    );
    for cycle in 0..2 {
        let enabled = call(&handle, "set_settings", r#"{"private_inference":true}"#);
        let observed = &enabled["private_inference_state"];
        // Any of the running family. This test is about the C ABI keeping a
        // proxy alive across calls and cycles, not about which account
        // answers -- and the destination is legitimately unknown here,
        // because nothing has read the proxy's status yet.
        assert!(
            matches!(
                observed["state"].as_str(),
                Some(
                    "running"
                        | "running_no_backends"
                        | "running_answered_elsewhere"
                        | "running_destination_unknown"
                )
            ),
            "{observed}"
        );
        let port = u16::try_from(observed["port"].as_u64().unwrap()).unwrap();
        // No ambient Tokio runtime exists here. tc_call has already returned,
        // so only the runtime owned by the daemon can answer this request.
        assert!(health_answers(port));
        if cycle == 0 {
            call(&handle, "set_settings", r#"{"private_inference":false}"#);
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let state = call(&handle, "get_settings", "{}")["private_inference_state"].clone();
                if state["state"] == "off" {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "owned proxy never finished stopping: {state}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(!root.join("ironwire/endpoint.json").exists());
        }
    }
    // The second cycle is stopped through the actual C teardown path.
    drop(handle);
    assert!(!root.join("ironwire/endpoint.json").exists());
}

#[test]
fn c_api_private_inference_outlives_call_and_cycles() {
    const CHILD: &str = "TC_PRIVATE_INFERENCE_TEST_CHILD";
    if let Some(root) = std::env::var_os(CHILD) {
        exercise_child(Path::new(&root));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("ironwire");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        "[server]\nport = 0\n[updates]\ncheck = false\n",
    )
    .unwrap();
    // Configure only the child process, avoiding global environment mutation
    // in the test runner and any use of a developer's real proxy or sessions.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "c_api_private_inference_outlives_call_and_cycles",
            "--nocapture",
        ])
        .env(CHILD, root.path())
        .env("IRONWIRE_HOME", &home)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
