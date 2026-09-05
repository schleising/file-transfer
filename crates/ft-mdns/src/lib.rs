//! Browse `_ssh._tcp` via mDNS / Bonjour.

use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct DiscoveredHost {
    pub name: String,
    pub host: String,
    pub port: u16,
}

/// Snapshot of currently known `_ssh._tcp` services.
/// Dropping this shuts down the mDNS daemon thread.
pub struct Discovery {
    inner: Arc<Mutex<HashMap<String, DiscoveredHost>>>,
    daemon: ServiceDaemon,
}

impl Discovery {
    pub fn start() -> Result<Self> {
        let inner = Arc::new(Mutex::new(HashMap::new()));
        let daemon = ServiceDaemon::new().context("mdns daemon")?;
        let receiver = daemon
            .browse("_ssh._tcp.local.")
            .context("browse _ssh._tcp")?;
        let map = inner.clone();
        std::thread::Builder::new()
            .name("ft-mdns".into())
            .spawn(move || browse_loop(map, receiver))?;
        Ok(Self { inner, daemon })
    }

    pub fn hosts(&self) -> Vec<DiscoveredHost> {
        let guard = self.inner.lock().unwrap();
        let mut v: Vec<_> = guard.values().cloned().collect();
        v.sort_by_key(|a| a.name.to_lowercase());
        v
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}

fn browse_loop(
    map: Arc<Mutex<HashMap<String, DiscoveredHost>>>,
    receiver: mdns_sd::Receiver<ServiceEvent>,
) {
    loop {
        match receiver.recv() {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let fullname = info.get_fullname().to_string();
                let host = info.get_hostname().trim_end_matches('.').to_string();
                let short = fullname.split('.').next().unwrap_or(&fullname).to_string();
                map.lock().unwrap().insert(
                    fullname,
                    DiscoveredHost {
                        name: short,
                        host,
                        port: info.get_port(),
                    },
                );
            }
            Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                map.lock().unwrap().remove(&fullname);
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
}
