//! Test-only child process that exercises real endpoint publication and signed IPC.
//! This models the source-derived protocol; it is not a HoloNear implementation.
use super::{LocalWorkerConfig, worker_protocol as wire};
use ring::signature::{self, KeyPair};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;

pub(super) fn config(root: &Path) -> LocalWorkerConfig {
    use std::os::unix::fs::PermissionsExt;
    let executable = std::env::current_exe().unwrap();
    let quoted = executable.to_str().unwrap().replace('\'', "'\\''");
    let script = format!(
        "#!/bin/sh\nexec '{quoted}' --ignored --exact compute::test_worker::run_fixture --nocapture\n"
    );
    let binary = root.join("signed-test-worker");
    std::fs::write(&binary, script.as_bytes()).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    LocalWorkerConfig {
        binary,
        expected_sha256: hex::encode(Sha256::digest(script.as_bytes())),
        coordinator: "ws://127.0.0.1:9999".into(),
        startup_timeout_secs: 5,
    }
}

#[test]
#[ignore = "invoked only as a child by the signed worker lifecycle tests"]
fn run_fixture() {
    let home = std::path::PathBuf::from(std::env::var_os("HOLONEAR_HOME").unwrap());
    let seed = hex::decode(std::env::var(wire::CREDENTIAL_ENV).unwrap()).unwrap();
    let key = signature::Ed25519KeyPair::from_seed_unchecked(&seed).unwrap();
    let instance: [u8; 32] = key.public_key().as_ref().try_into().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let node = home.join("node");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(node.join("worker.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let endpoint = serde_json::json!({
        "version": wire::VERSION, "instance": instance,
        "address": listener.local_addr().unwrap(),
    });
    std::fs::write(
        node.join("endpoint.tmp"),
        serde_json::to_vec(&endpoint).unwrap(),
    )
    .unwrap();
    std::fs::rename(node.join("endpoint.tmp"), node.join("worker-endpoint.json")).unwrap();
    for stream in listener.incoming() {
        let mut stream = stream.unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut header = [0; 4];
        stream.read_exact(&mut header).unwrap();
        let len = u32::from_be_bytes(header) as usize;
        assert!((1..=wire::MAX_FRAME_BYTES).contains(&len));
        let mut bytes = vec![0; len];
        stream.read_exact(&mut bytes).unwrap();
        let request: wire::SignedRequest = serde_json::from_slice(&bytes).unwrap();
        let mut signed = b"holonear-worker-request-v0\0".to_vec();
        signed.extend(serde_json::to_vec(&request.body).unwrap());
        signature::UnparsedPublicKey::new(&signature::ED25519, instance)
            .verify(&signed, &request.signature)
            .unwrap();
        assert_eq!(request.body.version, wire::VERSION);
        let drain = request.body.command == wire::Command::Drain;
        let state = if drain {
            wire::State::Draining
        } else if node.join("serve").exists() {
            wire::State::Serving
        } else {
            wire::State::Training
        };
        let body = wire::Response {
            version: wire::VERSION,
            instance,
            nonce: request.body.nonce,
            status: wire::Status {
                state,
                admission: wire::Admission::Assigned,
                drain: if drain {
                    wire::DrainOutcome::Acknowledged
                } else {
                    wire::DrainOutcome::NotRequested
                },
                free_mem_bytes: 1 << 30,
            },
        };
        let mut signed = b"holonear-worker-response-v0\0".to_vec();
        signed.extend(serde_json::to_vec(&body).unwrap());
        let response = wire::SignedResponse {
            body,
            signature: key.sign(&signed).as_ref().to_vec(),
        };
        let encoded = serde_json::to_vec(&response).unwrap();
        stream
            .write_all(&(encoded.len() as u32).to_be_bytes())
            .unwrap();
        stream.write_all(&encoded).unwrap();
        if drain {
            break;
        }
    }
}
