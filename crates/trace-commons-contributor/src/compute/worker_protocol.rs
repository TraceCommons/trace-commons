//! Holonear IPC v0 pinned to ef4e6e2479e8395f7d972d3342bad97851f2104e.
//! Field order and signature domains are part of the executable contract.

use ring::{
    rand::{SecureRandom, SystemRandom},
    signature::{self, KeyPair},
};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const VERSION: u32 = 0;
pub const MAX_FRAME_BYTES: usize = 65_536;
pub const CREDENTIAL_ENV: &str = "HOLONEAR_WORKER_CREDENTIAL";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Status,
    Drain,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Connecting,
    Joining,
    Training,
    Serving,
    Idle,
    Draining,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Admission {
    Unknown,
    AwaitingAssignment,
    Assigned,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DrainOutcome {
    NotRequested,
    Pending,
    Acknowledged,
    TimedOut,
    Unavailable,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub state: State,
    pub admission: Admission,
    pub drain: DrainOutcome,
    pub free_mem_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub nonce: [u8; 32],
    pub command: Command,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedRequest {
    pub body: Request,
    pub signature: Vec<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub instance: [u8; 32],
    pub nonce: [u8; 32],
    pub status: Status,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedResponse {
    pub body: Response,
    pub signature: Vec<u8>,
}

/// Ephemeral IPC capability, unrelated to payout or contributor credentials.
pub struct Credential {
    key: signature::Ed25519KeyPair,
}

impl Credential {
    pub fn from_seed(seed: &[u8; 32]) -> anyhow::Result<Self> {
        Ok(Self {
            key: signature::Ed25519KeyPair::from_seed_unchecked(seed)
                .map_err(|_| anyhow::anyhow!("worker-credential-invalid"))?,
        })
    }
    pub fn instance(&self) -> [u8; 32] {
        self.key
            .public_key()
            .as_ref()
            .try_into()
            .expect("Ed25519 public key length")
    }
    pub fn request(&self, command: Command) -> anyhow::Result<SignedRequest> {
        let mut nonce = [0; 32];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| anyhow::anyhow!("worker-random-failed"))?;
        self.request_with_nonce(command, nonce)
    }
    /// For deterministic conformance fixtures. Live calls use request().
    pub fn request_with_nonce(
        &self,
        command: Command,
        nonce: [u8; 32],
    ) -> anyhow::Result<SignedRequest> {
        let body = Request {
            version: VERSION,
            nonce,
            command,
        };
        let mut bytes = b"holonear-worker-request-v0\0".to_vec();
        bytes.extend(serde_json::to_vec(&body)?);
        Ok(SignedRequest {
            body,
            signature: self.key.sign(&bytes).as_ref().to_vec(),
        })
    }
    pub fn verify_response(&self, request: &SignedRequest, response: &SignedResponse) -> bool {
        if response.body.version != VERSION
            || response.body.instance != self.instance()
            || response.body.nonce != request.body.nonce
        {
            return false;
        }
        let Ok(body) = serde_json::to_vec(&response.body) else {
            return false;
        };
        let mut bytes = b"holonear-worker-response-v0\0".to_vec();
        bytes.extend(body);
        signature::UnparsedPublicKey::new(&signature::ED25519, self.instance())
            .verify(&bytes, &response.signature)
            .is_ok()
    }
    pub async fn exchange(&self, address: SocketAddr, command: Command) -> anyhow::Result<Status> {
        anyhow::ensure!(
            address.ip().is_loopback() && address.port() != 0,
            "worker-endpoint-invalid"
        );
        let deadline = match command {
            Command::Status => Duration::from_secs(2),
            Command::Drain => Duration::from_secs(10),
        };
        tokio::time::timeout(deadline, async {
            let request = self.request(command)?;
            let bytes = serde_json::to_vec(&request)?;
            let mut stream = tokio::net::TcpStream::connect(address).await?;
            stream.write_u32(bytes.len() as u32).await?;
            stream.write_all(&bytes).await?;
            let len = stream.read_u32().await? as usize;
            anyhow::ensure!((1..=MAX_FRAME_BYTES).contains(&len), "worker-frame-invalid");
            let mut bytes = vec![0; len];
            stream.read_exact(&mut bytes).await?;
            let response: SignedResponse = serde_json::from_slice(&bytes)
                .map_err(|_| anyhow::anyhow!("worker-response-invalid"))?;
            anyhow::ensure!(
                self.verify_response(&request, &response),
                "worker-authentication-failed"
            );
            Ok(response.body.status)
        })
        .await
        .map_err(|_| anyhow::anyhow!("worker-request-timeout"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_holonear_vectors_match_both_directions_and_reject_tampering() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/worker_ipc_v0.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let seed: [u8; 32] = hex::decode(case["seed_hex"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap();
            let credential = Credential::from_seed(&seed).unwrap();
            let request: SignedRequest = serde_json::from_value(case["request"].clone()).unwrap();
            let response: SignedResponse =
                serde_json::from_value(case["response"].clone()).unwrap();
            let actual = credential
                .request_with_nonce(request.body.command, request.body.nonce)
                .unwrap();
            assert_eq!(actual.signature, request.signature);
            assert_eq!(
                serde_json::to_string(&actual.body).unwrap(),
                case["request_body_json"].as_str().unwrap()
            );
            assert_eq!(
                serde_json::to_string(&response.body).unwrap(),
                case["response_body_json"].as_str().unwrap()
            );
            assert!(credential.verify_response(&actual, &response));
            let mut wrong = response.clone();
            wrong.body.version += 1;
            assert!(!credential.verify_response(&actual, &wrong));
            let mut wrong = response.clone();
            wrong.body.nonce[0] ^= 1;
            assert!(!credential.verify_response(&actual, &wrong));
            let mut wrong = response.clone();
            wrong.body.instance[0] ^= 1;
            assert!(!credential.verify_response(&actual, &wrong));
            let mut wrong = response.clone();
            wrong.body.status.free_mem_bytes += 1;
            assert!(!credential.verify_response(&actual, &wrong));
            let mut wrong = response.clone();
            wrong.signature = request.signature;
            assert!(!credential.verify_response(&actual, &wrong));
        }
    }

    #[tokio::test]
    async fn refuses_oversized_empty_malformed_and_disconnected_frames() {
        for frame in [0, MAX_FRAME_BYTES as u32 + 1, 2] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let peer = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let len = stream.read_u32().await.unwrap();
                let mut bytes = vec![0; len as usize];
                stream.read_exact(&mut bytes).await.unwrap();
                stream.write_u32(frame).await.unwrap();
                if frame == 2 {
                    stream.write_all(b"{}").await.unwrap();
                }
            });
            assert!(
                Credential::from_seed(&[3; 32])
                    .unwrap()
                    .exchange(address, Command::Status)
                    .await
                    .is_err()
            );
            peer.await.unwrap();
        }
        assert!(
            Credential::from_seed(&[3; 32])
                .unwrap()
                .exchange("192.0.2.1:8".parse().unwrap(), Command::Status)
                .await
                .is_err()
        );
    }
}
