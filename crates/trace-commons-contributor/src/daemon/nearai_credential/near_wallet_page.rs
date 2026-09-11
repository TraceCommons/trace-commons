//! Browser presentation and pinned connector assets. No Cloud credential reaches
//! this module. Wallet adapters are fetched without credentials and checked
//! against the reviewed manifest before the browser can execute them.

use std::collections::HashMap;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use ring::rand::{SecureRandom, SystemRandom};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::daemon::nearai_credential::near_wallet::{NearWalletChallenge, NearWalletError};

pub const ASSET_PREFIX: &str = "/near-ai/near-wallet/assets/";
const SDK: &str = include_str!("near_connect/near-connect-0.11.4.js");
const BROWSER: &str = include_str!("near_wallet_browser.js");
const MANIFEST: &str = include_str!("near_connect/manifest.json");
const MAX_ADAPTER_BYTES: usize = 2 * 1024 * 1024;
pub const ASSET_TIMEOUT: Duration = Duration::from_secs(15);

#[cfg(test)]
#[path = "near_wallet_page_tests.rs"]
mod tests;

#[derive(Deserialize)]
struct Manifest {
    wallets: Vec<Wallet>,
}

#[derive(Deserialize)]
struct Wallet {
    id: String,
    executor: String,
    sha256: String,
}

fn manifest() -> Result<Manifest, NearWalletError> {
    serde_json::from_str(MANIFEST).map_err(|_| NearWalletError::Unavailable)
}

/// Only bundled assets and the reviewed wallet IDs are routable. The SDK adds
/// a cache nonce; it never changes the upstream URL or the expected digest.
pub fn asset_path(request_path: &str) -> Option<&str> {
    let path = if let Some((path, query)) = request_path.split_once('?') {
        let nonce = query.strip_prefix("nonce=")?;
        if nonce.is_empty()
            || nonce.len() > 64
            || !nonce
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return None;
        }
        path
    } else {
        request_path
    };
    let name = path.strip_prefix(ASSET_PREFIX)?;
    if matches!(name, "connector.js" | "browser.js") {
        return Some(path);
    }
    let id = name.strip_suffix(".js")?;
    manifest()
        .ok()?
        .wallets
        .iter()
        .any(|wallet| wallet.id == id)
        .then_some(path)
}

pub fn render(challenge: &NearWalletChallenge) -> Result<String, NearWalletError> {
    let mut nonce = [0; 32];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| NearWalletError::Unavailable)?;
    let nonce = STANDARD.encode(nonce);
    let mut configuration = challenge.browser_configuration();
    let mut manifest: Value =
        serde_json::from_str(MANIFEST).map_err(|_| NearWalletError::Unavailable)?;
    let wallets = manifest["wallets"]
        .as_array_mut()
        .ok_or(NearWalletError::Unavailable)?;
    for wallet in wallets {
        let id = wallet["id"]
            .as_str()
            .ok_or(NearWalletError::Unavailable)?
            .to_owned();
        let hash = wallet["sha256"]
            .as_str()
            .ok_or(NearWalletError::Unavailable)?
            .to_owned();
        wallet["executor"] = Value::String(format!("{ASSET_PREFIX}{id}.js"));
        // The SDK executes cached code before checking its source. A fresh
        // namespace per page prevents reuse from a prior owner of this port;
        // the digest still identifies the bytes accepted by the native server.
        wallet["version"] = Value::String(format!("{hash}.{nonce}"));
        if let Some(object) = wallet.as_object_mut() {
            object.remove("sha256");
        }
    }
    configuration["manifest"] = manifest;
    configuration["cspNonce"] = Value::String(nonce.clone());
    configuration["copy"] = crate::private_inference_copy::wallet_browser_copy();
    // Escaping '<' makes the JSON script safe even if upstream metadata contains
    // an HTML closing tag. It does not alter the connector's parsed strings.
    let configuration = configuration.to_string().replace('<', "\\u003c");
    let notice = crate::private_inference_copy::CREDENTIAL_WALLET_NOTICE;
    let copy = crate::private_inference_copy::wallet_browser_copy();
    let title = copy["title"].as_str().ok_or(NearWalletError::Unavailable)?;
    let choose = copy["choose"]
        .as_str()
        .ok_or(NearWalletError::Unavailable)?;
    let unavailable = copy["refused"]
        .as_str()
        .ok_or(NearWalletError::Unavailable)?;
    let cancel = copy["cancel"]
        .as_str()
        .ok_or(NearWalletError::Unavailable)?;
    // HOT and NEAR Mobile render inline button handlers inside their sandbox.
    // Permit only their six pinned handler bodies after the SDK's selector
    // rewrite. The real-adapter browser regression holds these hashes and
    // rejects any other handler; script elements still require this nonce.
    Ok(format!(
        r#"<!doctype html><html lang="en"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="referrer" content="no-referrer">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-{nonce}'; script-src-attr 'unsafe-hashes' 'sha256-hiR4AYasgB4oeFRysx6JvOoXEm7Sv2Iagfqxuhw6AHI=' 'sha256-45eAs6mQzW89Jy+zv9ZwmOA3VrRwV6eYjE60xCF6Kbo=' 'sha256-Yp9s+EjWLQFQQFikRHyY3SumlPFV6cYRjrvX//CQetQ=' 'sha256-SCpVD+RLbNF3saIG8m2Dd4tEpw9OgUZHA3yFmrPwjaU=' 'sha256-1Th1PhaH/QoNIM7rQLV4eOOSDnF+zgIcKqIzVyz80Cc=' 'sha256-K38ZFa/HjxTuewSAQJ4RFzIk6/DLmUXsFEYGFWPskQo='; style-src 'unsafe-inline'; img-src https: data:; connect-src 'self' https:; frame-src 'self' hotwallet: near-mobile-wallet:; base-uri 'none'; form-action 'none'">
<title>{title}</title>
<style>
:root {{color-scheme:light dark;font-family:system-ui,sans-serif;color:#202620;background:#fafbf9;-webkit-font-smoothing:antialiased}}
* {{box-sizing:border-box}} body {{margin:0;padding:32px 24px}} main {{max-width:640px;margin:32px auto;padding:32px;border:1px solid #d8dfd9;border-radius:12px}}
h1 {{font-size:24px;line-height:1.2;letter-spacing:-.02em;font-weight:600;margin:0 0 24px}} p {{font-size:16px;line-height:1.5;margin:0 0 24px}}
.actions {{display:flex;flex-wrap:wrap;gap:16px}} button {{min-height:44px;padding:8px 16px;border:1px solid #647267;border-radius:8px;font:inherit;cursor:pointer}} #connect {{background:#087e65;color:#fff;border-color:#087e65}} button:disabled {{opacity:.6;cursor:default}} button:focus-visible {{outline:2px solid #087e65;outline-offset:4px}}
#status {{margin:24px 0 0}} @media(prefers-color-scheme:dark) {{:root {{background:#171c19;color:#f1f5f1}} main {{border-color:#3a463e}} #connect {{background:#62d8b7;color:#10231b;border-color:#62d8b7}}}} @media(max-width:480px) {{body {{padding:16px}} main {{margin:16px auto;padding:24px}}}}
</style></head><body><main><h1>{title}</h1><p>{notice}</p><div class="actions"><button id="connect">{choose}</button><button id="cancel">{cancel}</button></div><p id="status" role="status" aria-live="polite">{unavailable}</p></main>
<script id="challenge" type="application/json">{configuration}</script>
<script nonce="{nonce}" src="{ASSET_PREFIX}connector.js"></script><script nonce="{nonce}" src="{ASSET_PREFIX}browser.js"></script>
</body></html>"#
    ))
}

pub struct Assets {
    client: reqwest::Client,
    cached: HashMap<String, String>,
}

impl Assets {
    pub fn new() -> Result<Self, NearWalletError> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(ASSET_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .build()
                .map_err(|_| NearWalletError::Unavailable)?,
            cached: HashMap::new(),
        })
    }

    pub async fn load(&mut self, path: &str) -> Result<&str, NearWalletError> {
        let name = asset_path(path)
            .and_then(|path| path.strip_prefix(ASSET_PREFIX))
            .ok_or(NearWalletError::Invalid)?;
        match name {
            "connector.js" => return Ok(SDK),
            "browser.js" => return Ok(BROWSER),
            _ => {}
        }
        let id = name.strip_suffix(".js").ok_or(NearWalletError::Invalid)?;
        if !self.cached.contains_key(id) {
            let wallet = manifest()?
                .wallets
                .into_iter()
                .find(|wallet| wallet.id == id)
                .ok_or(NearWalletError::Invalid)?;
            let body = fetch_script(&self.client, &wallet.executor, &wallet.sha256).await?;
            self.cached.insert(id.to_owned(), body);
        }
        self.cached
            .get(id)
            .map(String::as_str)
            .ok_or(NearWalletError::Unavailable)
    }
}

async fn fetch_script(
    client: &reqwest::Client,
    url: &str,
    digest: &str,
) -> Result<String, NearWalletError> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| NearWalletError::Unavailable)?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_ADAPTER_BYTES as u64)
    {
        return Err(NearWalletError::Unavailable);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| NearWalletError::Unavailable)?
    {
        if chunk.len() > MAX_ADAPTER_BYTES.saturating_sub(bytes.len()) {
            return Err(NearWalletError::Unavailable);
        }
        bytes.extend_from_slice(&chunk);
    }
    if format!("{:x}", Sha256::digest(&bytes)) != digest {
        return Err(NearWalletError::Unavailable);
    }
    String::from_utf8(bytes).map_err(|_| NearWalletError::Unavailable)
}
