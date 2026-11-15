//! Browse `_ssh._tcp` via mDNS / Bonjour.

use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
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
    ready: Arc<AtomicBool>,
    daemon: ServiceDaemon,
}

impl Discovery {
    pub fn start() -> Result<Self> {
        let inner = Arc::new(Mutex::new(HashMap::new()));
        let ready = Arc::new(AtomicBool::new(false));
        let daemon = ServiceDaemon::new().context("mdns daemon")?;
        let receiver = daemon
            .browse("_ssh._tcp.local.")
            .context("browse _ssh._tcp")?;
        let map = inner.clone();
        let ready_flag = ready.clone();
        std::thread::Builder::new()
            .name("ft-mdns".into())
            .spawn(move || browse_loop(map, ready_flag, receiver))?;
        Ok(Self {
            inner,
            ready,
            daemon,
        })
    }

    pub fn hosts(&self) -> Vec<DiscoveredHost> {
        let mut v: Vec<_> = lock_map(&self.inner).values().cloned().collect();
        v.sort_by_key(|a| a.name.to_lowercase());
        v
    }

    /// True after the browse has delivered its first event, including an empty result.
    pub fn has_reported(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}

fn browse_loop(
    map: Arc<Mutex<HashMap<String, DiscoveredHost>>>,
    ready: Arc<AtomicBool>,
    receiver: mdns_sd::Receiver<ServiceEvent>,
) {
    while let Ok(event) = receiver.recv() {
        ready.store(true, Ordering::Relaxed);
        match event {
            ServiceEvent::ServiceResolved(info) => {
                let fullname = info.get_fullname().to_string();
                let host = info.get_hostname().trim_end_matches('.').to_string();
                let short = match fullname.split('.').next() {
                    Some(name) if !name.is_empty() => name.to_string(),
                    _ => fullname.clone(),
                };
                lock_map(&map).insert(
                    fullname,
                    DiscoveredHost {
                        name: short,
                        host,
                        port: info.get_port(),
                    },
                );
            }
            ServiceEvent::ServiceRemoved(_, fullname) => {
                lock_map(&map).remove(&fullname);
            }
            _ => {}
        }
    }
}

fn lock_map(
    map: &Mutex<HashMap<String, DiscoveredHost>>,
) -> std::sync::MutexGuard<'_, HashMap<String, DiscoveredHost>> {
    match map.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
