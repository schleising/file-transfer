//! Browse `_ssh._tcp` via mDNS / Bonjour.

use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DiscoveredHost {
    pub name: String,
    pub host: String,
    pub port: u16,
}

/// Snapshot of currently known `_ssh._tcp` services.
#[derive(Clone, Default)]
pub struct Discovery {
    inner: Arc<Mutex<HashMap<String, DiscoveredHost>>>,
}

impl Discovery {
    pub fn start() -> Result<Self> {
        let discovery = Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        };
        let map = discovery.inner.clone();
        std::thread::Builder::new()
            .name("ft-mdns".into())
            .spawn(move || {
                if let Err(e) = browse_loop(map) {
                    eprintln!("mdns browse ended: {e:#}");
                }
            })?;
        Ok(discovery)
    }

    pub fn hosts(&self) -> Vec<DiscoveredHost> {
        let guard = self.inner.lock().unwrap();
        let mut v: Vec<_> = guard.values().cloned().collect();
        v.sort_by_key(|a| a.name.to_lowercase());
        v
    }
}

fn browse_loop(map: Arc<Mutex<HashMap<String, DiscoveredHost>>>) -> Result<()> {
    let daemon = ServiceDaemon::new().context("mdns daemon")?;
    let receiver = daemon
        .browse("_ssh._tcp.local.")
        .context("browse _ssh._tcp")?;

    loop {
        match receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let fullname = info.get_fullname().to_string();
                let host = info.get_hostname().trim_end_matches('.').to_string();
                let short = fullname.split('.').next().unwrap_or(&fullname).to_string();
                let entry = DiscoveredHost {
                    name: short,
                    host,
                    port: info.get_port(),
                };
                map.lock().unwrap().insert(fullname, entry);
            }
            Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                map.lock().unwrap().remove(&fullname);
            }
            Ok(_) => {}
            Err(flume::RecvTimeoutError::Timeout) => {}
            Err(flume::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}
