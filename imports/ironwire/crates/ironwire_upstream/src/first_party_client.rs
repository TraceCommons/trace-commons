//! Login count must not multiply TLS root stores and connection pools.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

const MAX_CLIENTS: usize = 16;

#[derive(Default)]
struct Clients {
    by_timeout: HashMap<u64, Arc<reqwest::Client>>,
}

impl Clients {
    fn get(&mut self, timeout_secs: u64) -> reqwest::Result<Arc<reqwest::Client>> {
        if let Some(client) = self.by_timeout.get(&timeout_secs) {
            return Ok(Arc::clone(client));
        }
        let client = Arc::new(
            reqwest::Client::builder()
                // Auth and account headers belong to each request, never this
                // shared pool. Redirects cannot authorize another destination.
                .redirect(reqwest::redirect::Policy::none())
                // Streaming inference has no total timeout.
                .read_timeout(Duration::from_secs(timeout_secs))
                .connect_timeout(Duration::from_secs(15))
                .pool_idle_timeout(Duration::from_secs(90))
                .build()?,
        );
        if self.by_timeout.len() < MAX_CLIENTS {
            self.by_timeout.insert(timeout_secs, Arc::clone(&client));
        }
        Ok(client)
    }
}

pub(crate) fn build(timeout_secs: u64) -> reqwest::Result<reqwest::Client> {
    static CLIENTS: OnceLock<Mutex<Clients>> = OnceLock::new();
    let mut clients = CLIENTS
        .get_or_init(|| Mutex::new(Clients::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(clients.get(timeout_secs)?.as_ref().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thousand_logins_with_the_same_timeout_share_one_client() {
        let mut clients = Clients::default();
        let first = clients.get(60).expect("client");
        for _ in 0..1024 {
            assert!(Arc::ptr_eq(&first, &clients.get(60).expect("client")));
        }
        assert_eq!(clients.by_timeout.len(), 1);
    }

    #[test]
    fn unusual_timeouts_do_not_grow_the_shared_cache_without_bound() {
        let mut clients = Clients::default();
        for timeout in 1..=MAX_CLIENTS as u64 {
            clients.get(timeout).expect("client");
        }
        let first = clients.get(100).expect("uncached client");
        let next = clients.get(100).expect("another uncached client");
        assert!(!Arc::ptr_eq(&first, &next));
        assert_eq!(clients.by_timeout.len(), MAX_CLIENTS);
    }
}
