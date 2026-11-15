//! Toolkit-agnostic application state and background transfer jobs.

use crate::util::{
    discovery_matches, endpoints_match, first_free_host_color, folder_display_name, format_bytes,
    host_color, parse_port_field, truncate_err,
};
use anyhow::Result;
use chrono::Utc;
use ft_exec::{self, DirEntry, HostRef, Progress, TransferPlan};
use ft_mdns::{DiscoveredHost, Discovery};
use ft_store::{Computer, Location, LocationKind, Store};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

pub enum BgMsg {
    ListResult(Result<Vec<DirEntry>, String>),
    BrowseList {
        path: PathBuf,
        result: Result<Vec<DirEntry>, String>,
    },
    Mkdir {
        parent: PathBuf,
        name: String,
        result: Result<(), String>,
    },
    Preflight {
        gen: u64,
        result: Result<TransferPlan, String>,
    },
    Progress(Progress),
    TransferDone(Result<(u64, bool), String>),
    AutoReset {
        gen: u64,
    },
}

#[derive(Clone)]
struct BgTx {
    tx: Sender<BgMsg>,
    pending: Arc<AtomicBool>,
}

impl BgTx {
    fn pair() -> (Self, Receiver<BgMsg>) {
        let (tx, rx) = mpsc::channel();
        (
            Self {
                tx,
                pending: Arc::new(AtomicBool::new(false)),
            },
            rx,
        )
    }

    fn send(&self, msg: BgMsg) {
        let _ = self.tx.send(msg);
        self.pending.store(true, Ordering::Release);
    }

    fn pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }

    fn clear_pending(&self) {
        self.pending.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BrowseTarget {
    Source,
    Dest,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Source,
    Dest,
}

#[derive(Clone, PartialEq, Eq)]
pub enum AccessCheck {
    Untested,
    Testing,
    Accessible {
        message: String,
        file_count: u64,
        bytes_total: Option<u64>,
    },
    Inaccessible(String),
}

#[derive(Clone, PartialEq, Eq)]
pub struct CopyItem {
    pub name: String,
    pub is_dir: bool,
}

impl AccessCheck {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Untested => "idle",
            Self::Testing => "testing",
            Self::Accessible { .. } => "ok",
            Self::Inaccessible(_) => "err",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Untested => "Untested",
            Self::Testing => "Testing",
            Self::Accessible { .. } => "Accessible",
            Self::Inaccessible(_) => "Inaccessible",
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            Self::Untested | Self::Testing => self.label(),
            Self::Accessible { message, .. } => message,
            Self::Inaccessible(msg) => msg,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum LastTransfer {
    Complete { bytes: u64 },
    Failed(String),
    Cancelled,
}

impl LastTransfer {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Complete { .. } => "ok",
            Self::Failed(_) => "err",
            Self::Cancelled => "idle",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Complete { .. } => "Complete",
            Self::Failed(_) => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::Complete { bytes } => {
                format!("Last transfer complete ({})", format_bytes(*bytes))
            }
            Self::Failed(msg) => format!("Last transfer failed: {msg}"),
            Self::Cancelled => "Last transfer cancelled".into(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavTab {
    Source,
    Files,
    Destination,
    Hosts,
}

impl NavTab {
    pub fn prev(self) -> Option<Self> {
        match self {
            NavTab::Source | NavTab::Hosts => None,
            NavTab::Files => Some(NavTab::Source),
            NavTab::Destination => Some(NavTab::Files),
        }
    }

    pub fn next(self) -> Option<Self> {
        match self {
            NavTab::Source => Some(NavTab::Files),
            NavTab::Files => Some(NavTab::Destination),
            NavTab::Destination | NavTab::Hosts => None,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            NavTab::Source => "Source",
            NavTab::Files => "Files",
            NavTab::Destination => "Destination",
            NavTab::Hosts => "Hosts",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            NavTab::Source => "Choose a saved folder.",
            NavTab::Files => "Select files and folders to copy.",
            NavTab::Destination => "Choose where the files should go.",
            NavTab::Hosts => "Add, edit, or remove computers used for transfer.",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HostFormTarget {
    Existing(Uuid),
    New,
}

#[derive(Clone)]
pub struct HostForm {
    pub target: HostFormTarget,
    pub name: String,
    pub ssh: String,
    pub port: String,
    pub color: String,
    pub confirm_delete: bool,
    pub message: String,
}

pub struct FolderBrowser {
    pub target: BrowseTarget,
    pub computer_id: Uuid,
    pub current_path: PathBuf,
    pub path_edit: String,
    pub entries: Vec<DirEntry>,
    pub loading: bool,
    pub error: Option<String>,
    pub new_folder_name: String,
}

pub struct AppState {
    store: Store,
    discovery: Option<Discovery>,
    pub tab: NavTab,

    pub computers: Vec<Computer>,
    pub locations: Vec<Location>,

    pub source_computer: Option<Uuid>,
    pub source_location: Option<Uuid>,
    pub dest_computer: Option<Uuid>,
    pub dest_location: Option<Uuid>,

    pub entries: Vec<DirEntry>,
    pub selected: HashSet<String>,
    pub list_error: Option<String>,
    pub listing: bool,
    list_loaded_for: Option<Uuid>,

    pub access: AccessCheck,
    preflight_gen: u64,
    cached_plan: Option<TransferPlan>,
    pub progress: Progress,
    pub transferring: bool,
    pub status_line: String,
    pub last_transfer: Option<LastTransfer>,
    auto_reset_gen: u64,
    pub cancel: Arc<AtomicBool>,

    pub host_form: Option<HostForm>,

    pub folder_browser: Option<FolderBrowser>,

    bg: BgTx,
    rx: Receiver<BgMsg>,
}

impl AppState {
    pub fn new() -> Self {
        let (store, warning) = match Store::open_default() {
            Ok(store) => (store, None),
            Err(err) => match Store::open(":memory:") {
                Ok(store) => (
                    store,
                    Some(format!("Could not open the local store: {err:#}")),
                ),
                Err(mem_err) => {
                    eprintln!("File Transfer could not open a store: {err:#}; {mem_err:#}");
                    std::process::exit(1);
                }
            },
        };
        let (bg, rx) = BgTx::pair();
        let mut app = Self {
            store,
            discovery: None,
            tab: NavTab::Source,
            computers: vec![],
            locations: vec![],
            source_computer: None,
            source_location: None,
            dest_computer: None,
            dest_location: None,
            entries: vec![],
            selected: HashSet::new(),
            list_error: None,
            listing: false,
            list_loaded_for: None,
            access: AccessCheck::Untested,
            preflight_gen: 0,
            cached_plan: None,
            progress: Progress::default(),
            transferring: false,
            status_line: String::new(),
            last_transfer: None,
            auto_reset_gen: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            host_form: None,
            folder_browser: None,
            bg,
            rx,
        };
        app.reload_store();
        if let Some(local) = app.computers.iter().find(|c| c.is_local) {
            app.source_computer = Some(local.id);
            app.dest_computer = Some(local.id);
        }
        if let Some(warning) = warning {
            app.status_line = warning;
        }
        app
    }

    pub fn reload_store(&mut self) {
        self.computers = self.store.computers().unwrap_or_default();
        self.locations = self.store.locations().unwrap_or_default();
    }

    pub fn computer(&self, id: Uuid) -> Option<&Computer> {
        self.computers.iter().find(|c| c.id == id)
    }

    pub fn location(&self, id: Uuid) -> Option<&Location> {
        self.locations.iter().find(|l| l.id == id)
    }

    pub fn discovered_hosts(&self) -> Vec<DiscoveredHost> {
        self.discovery
            .as_ref()
            .map(|d| d.hosts())
            .unwrap_or_default()
    }

    pub fn last_transfer_kind(&self) -> &'static str {
        if self.transferring {
            "testing"
        } else {
            self.last_transfer
                .as_ref()
                .map(LastTransfer::kind)
                .unwrap_or("idle")
        }
    }

    pub fn last_transfer_label(&self) -> &'static str {
        if self.transferring {
            "In progress"
        } else {
            self.last_transfer
                .as_ref()
                .map(LastTransfer::label)
                .unwrap_or("None")
        }
    }

    pub fn last_transfer_detail(&self) -> String {
        if self.transferring {
            "Transfer in progress".into()
        } else {
            self.last_transfer
                .as_ref()
                .map(LastTransfer::detail)
                .unwrap_or_else(|| "No transfer yet".into())
        }
    }

    pub fn bg_pending(&self) -> bool {
        self.bg.pending()
    }

    pub fn bg_work_active(&self) -> bool {
        self.transferring
            || self.listing
            || matches!(self.access, AccessCheck::Testing)
            || self.folder_browser.as_ref().is_some_and(|b| b.loading)
    }

    fn ensure_discovery(&mut self) {
        if self.discovery.is_none() {
            self.discovery = Discovery::start().ok();
        }
    }

    fn leave_hosts(&mut self) {
        self.discovery = None;
        self.host_form = None;
    }

    fn enter_hosts(&mut self) {
        self.ensure_discovery();
        self.select_fallback_host();
    }

    pub fn discovery_pending(&self) -> bool {
        self.discovery
            .as_ref()
            .is_some_and(|discovery| !discovery.has_reported())
    }

    pub fn host_on_network(&self, computer_id: Uuid) -> bool {
        let Some(computer) = self.computer(computer_id) else {
            return false;
        };
        self.discovered_hosts().iter().any(|host| {
            discovery_matches(
                computer.bonjour_name.as_deref(),
                &computer.ssh_destination,
                computer.ssh_port,
                &host.name,
                &host.host,
                host.port,
            )
        })
    }

    pub fn unmatched_discoveries(&self) -> Vec<DiscoveredHost> {
        self.discovered_hosts()
            .into_iter()
            .filter(|host| {
                !self.computers.iter().any(|computer| {
                    discovery_matches(
                        computer.bonjour_name.as_deref(),
                        &computer.ssh_destination,
                        computer.ssh_port,
                        &host.name,
                        &host.host,
                        host.port,
                    )
                })
            })
            .collect()
    }

    pub fn location_groups(&self) -> Vec<(Computer, Vec<Location>)> {
        let mut by_host: HashMap<Uuid, Vec<Location>> = HashMap::new();
        for loc in &self.locations {
            by_host
                .entry(loc.computer_id)
                .or_default()
                .push(loc.clone());
        }
        for locs in by_host.values_mut() {
            locs.sort_by_key(|l| l.sort_order);
        }
        self.computers
            .iter()
            .map(|c| {
                let locs = by_host.get(&c.id).cloned().unwrap_or_default();
                (c.clone(), locs)
            })
            .collect()
    }

    fn side_location(&self, side: Side) -> Option<&Location> {
        let id = match side {
            Side::Source => self.source_location,
            Side::Dest => self.dest_location,
        }?;
        self.location(id)
    }

    pub fn side_host_name(&self, side: Side) -> String {
        self.side_location(side)
            .and_then(|loc| self.computer(loc.computer_id))
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "—".into())
    }

    pub fn side_folder_name(&self, side: Side) -> String {
        self.side_location(side)
            .map(|loc| folder_display_name(&loc.path, &loc.name))
            .unwrap_or_else(|| "—".into())
    }

    pub fn side_folder_title(&self, side: Side) -> String {
        self.side_location(side)
            .map(|loc| loc.path.display().to_string())
            .unwrap_or_default()
    }

    pub fn copy_items(&self) -> Vec<CopyItem> {
        let mut items: Vec<CopyItem> = self
            .selected
            .iter()
            .map(|name| {
                let is_dir = self
                    .entries
                    .iter()
                    .find(|e| e.name == *name)
                    .map(|e| e.is_dir)
                    .unwrap_or(false);
                CopyItem {
                    name: name.clone(),
                    is_dir,
                }
            })
            .collect();
        items.sort_by(|a, b| a.name.cmp(&b.name));
        items
    }

    pub fn copy_stats_label(&self) -> String {
        if matches!(self.access, AccessCheck::Testing) {
            return "Calculating…".into();
        }
        let (file_count, bytes_total) = match &self.access {
            AccessCheck::Accessible {
                file_count,
                bytes_total,
                ..
            } => (*file_count, *bytes_total),
            _ => {
                let mut file_count = 0u64;
                let mut bytes = 0u64;
                for e in &self.entries {
                    if self.selected.contains(&e.name) && !e.is_dir {
                        file_count += 1;
                        bytes += e.size;
                    }
                }
                (file_count, Some(bytes))
            }
        };
        let files = if file_count == 1 {
            "1 file".to_string()
        } else {
            format!("{file_count} files")
        };
        match bytes_total {
            Some(n) => format!("{files} · {}", format_bytes(n)),
            None => files,
        }
    }

    pub fn source_folder_label(&self) -> Option<String> {
        let loc = self.source_location.and_then(|id| self.location(id))?;
        let host = self
            .computer(loc.computer_id)
            .map(|c| c.name.as_str())
            .unwrap_or("?");
        let folder = folder_display_name(&loc.path, &loc.name);
        Some(format!("{host} — {folder}"))
    }

    pub fn ensure_location(&mut self, computer_id: Uuid, path: PathBuf) -> Uuid {
        if let Some(existing) = self
            .locations
            .iter()
            .find(|l| l.computer_id == computer_id && l.path == path)
        {
            return existing.id;
        }
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let sort_order = self
            .locations
            .iter()
            .filter(|l| l.computer_id == computer_id)
            .map(|l| l.sort_order)
            .max()
            .unwrap_or(-1)
            + 1;
        let now = Utc::now();
        let loc = Location {
            id: Uuid::new_v4(),
            computer_id,
            name,
            path,
            kind: LocationKind::Either,
            sort_order,
            created_at: now,
            updated_at: now,
        };
        let id = loc.id;
        let _ = self.store.upsert_location(&loc);
        self.reload_store();
        id
    }

    pub fn open_folder_browse(&mut self, computer_id: Uuid, target: BrowseTarget) {
        if self.computer(computer_id).is_none() {
            return;
        }

        let current = self
            .locations
            .iter()
            .find(|l| l.computer_id == computer_id)
            .map(|l| l.path.clone())
            .unwrap_or_else(|| PathBuf::from("~"));

        self.folder_browser = Some(FolderBrowser {
            target,
            computer_id,
            current_path: current.clone(),
            path_edit: current.to_string_lossy().into_owned(),
            entries: vec![],
            loading: true,
            error: None,
            new_folder_name: String::new(),
        });
        self.refresh_folder_browser();
    }

    pub fn apply_browsed_folder(&mut self, computer_id: Uuid, target: BrowseTarget, path: PathBuf) {
        match target {
            BrowseTarget::Source => {
                let id = self.ensure_location(computer_id, path);
                self.apply_location_selection(Some(id), Side::Source);
                self.start_list();
            }
            BrowseTarget::Dest => {
                let id = self.ensure_location(computer_id, path);
                self.apply_location_selection(Some(id), Side::Dest);
            }
        }
    }

    pub fn refresh_folder_browser(&mut self) {
        let Some(browser) = &self.folder_browser else {
            return;
        };
        let computer_id = browser.computer_id;
        let mut path = browser.current_path.clone();
        let Some(c) = self.computer(computer_id).cloned() else {
            return;
        };
        if let Some(browser) = &mut self.folder_browser {
            browser.loading = true;
            browser.error = None;
        }
        let bg = self.bg.clone();
        std::thread::spawn(move || {
            let host = AppState::host_ref(&c);
            if path == Path::new("~") {
                path = match ft_exec::remote_home(&host) {
                    Ok(h) => h,
                    Err(e) => {
                        bg.send(BgMsg::BrowseList {
                            path: PathBuf::from("~"),
                            result: Err(format!("{e:#}")),
                        });
                        return;
                    }
                };
            }
            let result = ft_exec::list_dir(&host, &path).map_err(|e| format!("{e:#}"));
            bg.send(BgMsg::BrowseList { path, result });
        });
    }

    pub fn browser_go_path(&mut self, path: PathBuf) {
        if let Some(browser) = &mut self.folder_browser {
            browser.current_path = path;
            browser.loading = true;
        }
        self.refresh_folder_browser();
    }

    pub fn browser_go_up(&mut self) {
        if let Some(browser) = &mut self.folder_browser {
            if let Some(parent) = browser.current_path.parent() {
                browser.current_path = parent.to_path_buf();
                if browser.current_path.as_os_str().is_empty() {
                    browser.current_path = PathBuf::from("/");
                }
            }
        }
        self.refresh_folder_browser();
    }

    pub fn browser_go_home(&mut self) {
        if let Some(browser) = &mut self.folder_browser {
            browser.current_path = PathBuf::from("~");
        }
        self.refresh_folder_browser();
    }

    pub fn browser_enter(&mut self, name: String) {
        if let Some(browser) = &mut self.folder_browser {
            browser.current_path = browser.current_path.join(name);
        }
        self.refresh_folder_browser();
    }

    pub fn browser_create_folder(&mut self) {
        let Some(browser) = &self.folder_browser else {
            return;
        };
        if browser.target != BrowseTarget::Dest || browser.loading {
            return;
        }
        let name = browser.new_folder_name.trim().to_string();
        if let Err(e) = valid_new_folder_name(&name) {
            if let Some(browser) = &mut self.folder_browser {
                browser.error = Some(e);
            }
            return;
        }
        if browser.entries.iter().any(|e| e.name == name) {
            if let Some(browser) = &mut self.folder_browser {
                browser.error = Some("A folder with that name already exists".into());
            }
            return;
        }
        let mut path = browser.current_path.clone();
        let Some(c) = self.computer(browser.computer_id).cloned() else {
            return;
        };
        self.note_user_activity();
        if let Some(browser) = &mut self.folder_browser {
            browser.loading = true;
            browser.error = None;
        }
        let bg = self.bg.clone();
        std::thread::spawn(move || {
            let host = AppState::host_ref(&c);
            if path == Path::new("~") {
                path = match ft_exec::remote_home(&host) {
                    Ok(h) => h,
                    Err(e) => {
                        bg.send(BgMsg::Mkdir {
                            parent: PathBuf::from("~"),
                            name,
                            result: Err(format!("{e:#}")),
                        });
                        return;
                    }
                };
            }
            let dest = path.join(&name);
            let result = ft_exec::mkdir(&host, &dest).map_err(|e| format!("{e:#}"));
            bg.send(BgMsg::Mkdir {
                parent: path,
                name,
                result,
            });
        });
    }

    pub fn browser_select(&mut self) {
        if self.selections_locked() {
            return;
        }
        if let Some(browser) = self.folder_browser.take() {
            self.apply_browsed_folder(browser.computer_id, browser.target, browser.current_path);
        }
    }

    pub fn host_ref(c: &Computer) -> HostRef {
        HostRef {
            is_local: c.is_local,
            ssh_destination: c.ssh_destination.clone(),
            ssh_port: c.ssh_port,
            identity_file: c.identity_file.clone(),
        }
    }

    pub fn poll_bg(&mut self) {
        self.bg.clear_pending();
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                BgMsg::ListResult(r) => {
                    self.listing = false;
                    match r {
                        Ok(e) => {
                            self.entries = e;
                            self.list_error = None;
                            self.list_loaded_for = self.source_location;
                        }
                        Err(e) => {
                            self.entries.clear();
                            self.list_error = Some(e);
                            self.list_loaded_for = self.source_location;
                        }
                    }
                }
                BgMsg::BrowseList { path, result } => {
                    if let Some(browser) = &mut self.folder_browser {
                        browser.loading = false;
                        browser.current_path = path.clone();
                        browser.path_edit = path.to_string_lossy().into_owned();
                        match result {
                            Ok(entries) => {
                                browser.entries =
                                    entries.into_iter().filter(|e| e.is_dir).collect();
                                browser.error = None;
                            }
                            Err(e) => {
                                browser.entries.clear();
                                browser.error = Some(e);
                            }
                        }
                    }
                }
                BgMsg::Mkdir {
                    parent,
                    name,
                    result,
                } => {
                    if self.folder_browser.is_none() {
                        continue;
                    }
                    match result {
                        Ok(()) => {
                            if let Some(browser) = &mut self.folder_browser {
                                browser.new_folder_name.clear();
                                browser.current_path = parent;
                            }
                            self.browser_enter(name);
                        }
                        Err(e) => {
                            if let Some(browser) = &mut self.folder_browser {
                                browser.loading = false;
                                browser.error = Some(e);
                            }
                        }
                    }
                }
                BgMsg::Preflight { gen, result } => {
                    if gen != self.preflight_gen {
                        continue;
                    }
                    match result {
                        Ok(plan) => {
                            self.access = AccessCheck::Accessible {
                                message: format!("OK — mode {:?}", plan.mode),
                                file_count: plan.file_count,
                                bytes_total: plan.bytes_total,
                            };
                            self.cached_plan = Some(plan);
                        }
                        Err(msg) => {
                            self.cached_plan = None;
                            self.access = AccessCheck::Inaccessible(msg);
                        }
                    }
                }
                BgMsg::Progress(p) => {
                    self.progress = p;
                }
                BgMsg::TransferDone(r) => {
                    self.transferring = false;
                    match r {
                        Ok((bytes, cancelled)) => {
                            let last = if cancelled {
                                LastTransfer::Cancelled
                            } else {
                                LastTransfer::Complete { bytes }
                            };
                            self.remember_job_end(last);
                        }
                        Err(e) => self.remember_job_end(LastTransfer::Failed(e)),
                    }
                }
                BgMsg::AutoReset { gen } => {
                    if gen == self.auto_reset_gen {
                        self.apply_auto_reset();
                    }
                }
            }
        }
    }

    pub fn start_list(&mut self) {
        let Some(cid) = self.source_computer else {
            return;
        };
        let Some(lid) = self.source_location else {
            return;
        };
        let Some(c) = self.computer(cid).cloned() else {
            return;
        };
        let Some(loc) = self.location(lid).cloned() else {
            return;
        };
        self.listing = true;
        self.list_error = None;
        self.list_loaded_for = None;
        let bg = self.bg.clone();
        std::thread::spawn(move || {
            let host = AppState::host_ref(&c);
            let res = ft_exec::list_dir(&host, &loc.path).map_err(|e| format!("{e:#}"));
            bg.send(BgMsg::ListResult(res));
        });
    }

    pub fn refresh_file_list(&mut self) {
        if self.selections_locked() {
            return;
        }
        self.list_loaded_for = None;
        self.start_list();
    }

    pub fn source_ready(&self) -> bool {
        self.source_location.is_some()
    }

    pub fn files_ready(&self) -> bool {
        !self.selected.is_empty()
    }

    pub fn selections_complete(&self) -> bool {
        self.source_ready() && self.files_ready() && self.dest_location.is_some()
    }

    pub fn transfer_ready(&self) -> bool {
        matches!(self.access, AccessCheck::Accessible { .. }) && !self.transferring
    }

    pub fn selections_locked(&self) -> bool {
        matches!(self.access, AccessCheck::Testing) || self.transferring
    }

    pub fn browse_on_host(&mut self, computer_id: Uuid, side: Side) {
        if self.selections_locked() {
            return;
        }
        let target = match side {
            Side::Source => BrowseTarget::Source,
            Side::Dest => BrowseTarget::Dest,
        };
        self.open_folder_browse(computer_id, target);
    }

    fn invalidate_preflight(&mut self) {
        self.preflight_gen = self.preflight_gen.wrapping_add(1);
        self.cached_plan = None;
        if self.transferring || !self.selections_complete() {
            self.access = AccessCheck::Untested;
            return;
        }
        self.spawn_preflight();
    }

    fn note_user_activity(&mut self) {
        self.auto_reset_gen = self.auto_reset_gen.wrapping_add(1);
    }

    fn schedule_auto_reset(&mut self) {
        self.auto_reset_gen = self.auto_reset_gen.wrapping_add(1);
        let gen = self.auto_reset_gen;
        let bg = self.bg.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            bg.send(BgMsg::AutoReset { gen });
        });
    }

    fn remember_job_end(&mut self, last: LastTransfer) {
        self.status_line = match &last {
            LastTransfer::Complete { bytes } => {
                #[cfg(target_os = "macos")]
                crate::macos::notify_transfers_complete();
                format!("Transfer complete ({})", format_bytes(*bytes))
            }
            LastTransfer::Failed(e) => format!("Transfer failed: {e}"),
            LastTransfer::Cancelled => "Cancelled".into(),
        };
        self.last_transfer = Some(last);
        self.schedule_auto_reset();
    }

    fn wipe_plan(&mut self) {
        self.apply_location_selection(None, Side::Source);
        self.apply_location_selection(None, Side::Dest);
        if let Some(local) = self.computers.iter().find(|c| c.is_local) {
            self.source_computer = Some(local.id);
            self.dest_computer = Some(local.id);
        } else {
            self.source_computer = None;
            self.dest_computer = None;
        }
        self.entries.clear();
        self.selected.clear();
        self.list_error = None;
        self.listing = false;
        self.list_loaded_for = None;
        self.preflight_gen = self.preflight_gen.wrapping_add(1);
        self.cached_plan = None;
        self.access = AccessCheck::Untested;
        self.progress = Progress::default();
        self.status_line.clear();
        self.cancel.store(false, Ordering::SeqCst);
        self.folder_browser = None;
        self.leave_hosts();
        self.tab = NavTab::Source;
    }

    fn apply_auto_reset(&mut self) {
        self.wipe_plan();
    }

    pub fn reset_transfer(&mut self) {
        if self.transferring {
            return;
        }
        self.note_user_activity();
        self.last_transfer = None;
        self.wipe_plan();
    }

    pub fn ensure_files_listed(&mut self) {
        let Some(lid) = self.source_location else {
            return;
        };
        if self.listing || self.list_loaded_for == Some(lid) {
            return;
        }
        self.start_list();
    }

    pub fn set_tab(&mut self, tab: NavTab) {
        self.note_user_activity();
        if tab == self.tab {
            if tab == NavTab::Files {
                self.ensure_files_listed();
            }
            return;
        }
        if self.tab == NavTab::Hosts {
            self.leave_hosts();
        }
        self.tab = tab;
        if tab == NavTab::Hosts {
            self.enter_hosts();
        } else if tab == NavTab::Files {
            self.ensure_files_listed();
        }
    }

    pub fn go_next(&mut self) {
        if let Some(next) = self.tab.next() {
            self.set_tab(next);
        }
    }

    pub fn go_back(&mut self) {
        if let Some(prev) = self.tab.prev() {
            self.note_user_activity();
            self.tab = prev;
        }
    }

    pub fn can_advance(&self) -> bool {
        match self.tab {
            NavTab::Source => self.source_ready(),
            NavTab::Files => self.files_ready(),
            NavTab::Destination => true,
            NavTab::Hosts => false,
        }
    }

    fn spawn_preflight(&mut self) {
        let inputs = match self.preflight_inputs() {
            Ok(inputs) => inputs,
            Err(e) => {
                self.access = AccessCheck::Inaccessible(e);
                return;
            }
        };
        self.access = AccessCheck::Testing;
        let gen = self.preflight_gen;
        let bg = self.bg.clone();
        std::thread::spawn(move || {
            let (source, dest, source_base, dest_base, relatives) = inputs;
            let result = ft_exec::plan_transfer(source, dest, source_base, dest_base, relatives)
                .and_then(|plan| {
                    ft_exec::preflight_start(&plan)?;
                    Ok(plan)
                })
                .map_err(|e| format!("{e:#}"));
            bg.send(BgMsg::Preflight { gen, result });
        });
    }

    fn preflight_inputs(
        &self,
    ) -> Result<(HostRef, HostRef, PathBuf, PathBuf, Vec<String>), String> {
        let sc = self
            .source_computer
            .and_then(|id| self.computer(id).cloned())
            .ok_or("Select source computer")?;
        let sl = self
            .source_location
            .and_then(|id| self.location(id).cloned())
            .ok_or("Select source location")?;
        let dc = self
            .dest_computer
            .and_then(|id| self.computer(id).cloned())
            .ok_or("Select destination computer")?;
        let dl = self
            .dest_location
            .and_then(|id| self.location(id).cloned())
            .ok_or("Select destination location")?;
        if self.selected.is_empty() {
            return Err("Select at least one file or folder".into());
        }
        Ok((
            Self::host_ref(&sc),
            Self::host_ref(&dc),
            sl.path,
            dl.path,
            self.selected.iter().cloned().collect(),
        ))
    }

    pub fn start_transfer(&mut self) {
        if !matches!(self.access, AccessCheck::Accessible { .. }) || self.transferring {
            return;
        }
        let Some(plan) = self.cached_plan.clone() else {
            self.status_line = "Access check has not finished".into();
            return;
        };

        self.note_user_activity();
        self.transferring = true;
        self.cancel.store(false, Ordering::SeqCst);
        self.progress = Progress {
            bytes_total: plan.bytes_total,
            files_total: (plan.file_count > 0).then_some(plan.file_count),
            ..Default::default()
        };
        self.status_line = "Transferring…".into();

        let bg = self.bg.clone();
        let cancel = self.cancel.clone();

        std::thread::spawn(move || {
            let on_prog = {
                let bg = bg.clone();
                move |p: Progress| {
                    bg.send(BgMsg::Progress(p));
                }
            };
            let result = ft_exec::run_transfer(&plan, cancel, on_prog);
            match result {
                Ok(r) => {
                    bg.send(BgMsg::TransferDone(Ok((r.bytes_transferred, r.cancelled))));
                }
                Err(e) => {
                    bg.send(BgMsg::TransferDone(Err(truncate_err(&format!("{e:#}")))));
                }
            }
        });
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn select_all_files(&mut self) {
        if self.selections_locked() {
            return;
        }
        self.note_user_activity();
        for e in &self.entries {
            self.selected.insert(e.name.clone());
        }
        self.invalidate_preflight();
    }

    pub fn clear_file_selection(&mut self) {
        if self.selections_locked() {
            return;
        }
        self.note_user_activity();
        self.selected.clear();
        self.invalidate_preflight();
    }

    pub fn toggle_file(&mut self, name: &str) {
        if self.selections_locked() {
            return;
        }
        self.note_user_activity();
        if self.selected.contains(name) {
            self.selected.remove(name);
        } else {
            self.selected.insert(name.to_string());
        }
        self.invalidate_preflight();
    }

    pub fn delete_location_tile(&mut self, id: Uuid) {
        if self.selections_locked() {
            return;
        }
        self.note_user_activity();
        if self.store.delete_location(id).is_err() {
            return;
        }
        if self.source_location == Some(id) {
            self.apply_location_selection(None, Side::Source);
        }
        if self.dest_location == Some(id) {
            self.apply_location_selection(None, Side::Dest);
        }
        self.reload_store();
    }

    pub fn reorder_location_tiles(
        &mut self,
        computer_id: Uuid,
        dragged_id: Uuid,
        before_id: Option<Uuid>,
    ) {
        if self.selections_locked() {
            return;
        }
        if dragged_id == before_id.unwrap_or(Uuid::nil()) {
            return;
        }
        let mut ordered: Vec<(i32, Uuid)> = self
            .locations
            .iter()
            .filter(|l| l.computer_id == computer_id)
            .map(|l| (l.sort_order, l.id))
            .collect();
        ordered.sort_by_key(|(order, _)| *order);
        let mut ids: Vec<Uuid> = ordered.into_iter().map(|(_, id)| id).collect();
        ids.retain(|id| *id != dragged_id);
        let pos = before_id
            .and_then(|id| ids.iter().position(|existing| *existing == id))
            .unwrap_or(ids.len());
        ids.insert(pos, dragged_id);
        if self.store.reorder_locations(computer_id, &ids).is_ok() {
            self.reload_store();
        }
    }

    pub fn select_host(&mut self, id: Uuid) {
        self.note_user_activity();
        if self.computer(id).is_some() {
            self.load_host_form(id);
        }
    }

    pub fn select_new_host(&mut self) {
        if self.selections_locked() {
            return;
        }
        self.note_user_activity();
        let color = first_free_host_color(self.stored_palette_colors());
        self.host_form = Some(HostForm {
            target: HostFormTarget::New,
            name: String::new(),
            ssh: String::new(),
            port: String::new(),
            color: color.to_string(),
            confirm_delete: false,
            message: String::new(),
        });
    }

    pub fn set_host_name(&mut self, value: String) {
        self.edit_host_field(|form| form.name = value);
    }

    pub fn set_host_ssh(&mut self, value: String) {
        self.edit_host_field(|form| form.ssh = value);
    }

    pub fn set_host_port(&mut self, value: String) {
        self.edit_host_field(|form| form.port = value);
    }

    pub fn set_host_color(&mut self, color: &str) {
        let color = color.to_string();
        self.edit_host_field(|form| form.color = color);
    }

    pub fn host_add_enabled(&self) -> bool {
        if self.selections_locked() {
            return false;
        }
        let Some(form) = &self.host_form else {
            return false;
        };
        if form.target != HostFormTarget::New {
            return false;
        }
        !form.name.trim().is_empty()
            && !form.ssh.trim().is_empty()
            && parse_port_field(&form.port).is_ok()
    }

    pub fn host_save_enabled(&self) -> bool {
        if self.selections_locked() {
            return false;
        }
        let Some(form) = self.host_form.clone() else {
            return false;
        };
        let HostFormTarget::Existing(id) = form.target else {
            return false;
        };
        let Some(computer) = self.computer(id).cloned() else {
            return false;
        };
        if form.name.trim().is_empty() || parse_port_field(&form.port).is_err() {
            return false;
        }
        if !computer.is_local && form.ssh.trim().is_empty() {
            return false;
        }
        host_record_dirty(&computer, &form)
    }

    pub fn add_discovered_host(&mut self, name: String, host: String, port: u16) {
        if self.selections_locked() {
            return;
        }
        self.note_user_activity();
        if let Some(existing) = self.computers.iter().find(|computer| {
            discovery_matches(
                computer.bonjour_name.as_deref(),
                &computer.ssh_destination,
                computer.ssh_port,
                &name,
                &host,
                port,
            )
        }) {
            let id = existing.id;
            self.load_host_form(id);
            return;
        }
        let now = Utc::now();
        let color = first_free_host_color(self.stored_palette_colors()).to_string();
        let computer = Computer {
            id: Uuid::new_v4(),
            name: name.clone(),
            ssh_destination: host,
            ssh_port: Some(port).filter(|&p| p != 22),
            identity_file: None,
            bonjour_name: Some(name),
            last_seen_at: Some(now),
            color: Some(color),
            is_local: false,
            created_at: now,
            updated_at: now,
        };
        if self.store.upsert_computer(&computer).is_err() {
            self.set_host_message("Could not save host.");
            return;
        }
        let id = computer.id;
        self.reload_store();
        self.load_host_form(id);
    }

    pub fn add_manual_host(&mut self) {
        if !self.host_add_enabled() {
            return;
        }
        self.note_user_activity();
        let Some(form) = self.host_form.clone() else {
            return;
        };
        let Ok(port) = parse_port_field(&form.port) else {
            return;
        };
        let name = form.name.trim().to_string();
        let ssh = form.ssh.trim().to_string();
        if self.endpoint_taken(&ssh, port, None) {
            self.set_host_message("That host is already saved.");
            return;
        }
        let now = Utc::now();
        let computer = Computer {
            id: Uuid::new_v4(),
            name,
            ssh_destination: ssh,
            ssh_port: port,
            identity_file: None,
            bonjour_name: None,
            last_seen_at: None,
            color: Some(form.color),
            is_local: false,
            created_at: now,
            updated_at: now,
        };
        if self.store.upsert_computer(&computer).is_err() {
            self.set_host_message("Could not save host.");
            return;
        }
        let id = computer.id;
        self.reload_store();
        self.load_host_form(id);
    }

    pub fn save_host(&mut self) {
        if !self.host_save_enabled() {
            return;
        }
        self.note_user_activity();
        let Some(form) = self.host_form.clone() else {
            return;
        };
        let HostFormTarget::Existing(id) = form.target else {
            return;
        };
        let Some(mut computer) = self.computer(id).cloned() else {
            return;
        };
        let Ok(port) = parse_port_field(&form.port) else {
            return;
        };
        let name = form.name.trim().to_string();
        if computer.is_local {
            computer.name = name;
            computer.color = Some(form.color);
        } else {
            let ssh = form.ssh.trim().to_string();
            if self.endpoint_taken(&ssh, port, Some(id)) {
                self.set_host_message("That host is already saved.");
                return;
            }
            computer.name = name;
            computer.ssh_destination = ssh;
            computer.ssh_port = port;
            computer.color = Some(form.color);
        }
        computer.updated_at = Utc::now();
        if self.store.upsert_computer(&computer).is_err() {
            self.set_host_message("Could not save host.");
            return;
        }
        self.reload_store();
        self.load_host_form(id);
    }

    pub fn begin_delete_host(&mut self) {
        if self.selections_locked() {
            return;
        }
        self.note_user_activity();
        let Some(form) = &self.host_form else {
            return;
        };
        let HostFormTarget::Existing(id) = form.target else {
            return;
        };
        let local = self.computer(id).is_some_and(|computer| computer.is_local);
        if local {
            return;
        }
        if let Some(form) = &mut self.host_form {
            form.confirm_delete = true;
            form.message.clear();
        }
    }

    pub fn cancel_delete_host(&mut self) {
        self.note_user_activity();
        if let Some(form) = &mut self.host_form {
            form.confirm_delete = false;
        }
    }

    pub fn confirm_delete_host(&mut self) {
        if self.selections_locked() {
            return;
        }
        self.note_user_activity();
        let Some(form) = self.host_form.clone() else {
            return;
        };
        let HostFormTarget::Existing(id) = form.target else {
            return;
        };
        if !form.confirm_delete {
            return;
        }
        let clear_source = self
            .source_location
            .and_then(|loc_id| self.location(loc_id))
            .is_some_and(|loc| loc.computer_id == id);
        let clear_dest = self
            .dest_location
            .and_then(|loc_id| self.location(loc_id))
            .is_some_and(|loc| loc.computer_id == id);
        let removed = self.store.delete_computer(id).unwrap_or_default();
        if !removed {
            self.set_host_message("Could not delete host.");
            return;
        }
        self.reload_store();
        if clear_source {
            self.apply_location_selection(None, Side::Source);
        }
        if clear_dest {
            self.apply_location_selection(None, Side::Dest);
        }
        if self.source_computer == Some(id) {
            self.source_computer = self.local_computer_id();
        }
        if self.dest_computer == Some(id) {
            self.dest_computer = self.local_computer_id();
        }
        self.select_fallback_host();
    }

    fn edit_host_field(&mut self, edit: impl FnOnce(&mut HostForm)) {
        if self.selections_locked() {
            return;
        }
        self.note_user_activity();
        if let Some(form) = &mut self.host_form {
            edit(form);
            form.confirm_delete = false;
            form.message.clear();
        }
    }

    fn set_host_message(&mut self, message: &str) {
        if let Some(form) = &mut self.host_form {
            form.message = message.to_string();
        }
    }

    fn stored_palette_colors(&self) -> Vec<&'static str> {
        self.computers
            .iter()
            .filter_map(|computer| {
                let stored = computer.color.as_deref()?;
                crate::util::HOST_PALETTE
                    .iter()
                    .copied()
                    .find(|color| stored.eq_ignore_ascii_case(color))
            })
            .collect()
    }

    fn endpoint_taken(&self, ssh: &str, port: Option<u16>, except: Option<Uuid>) -> bool {
        self.computers.iter().any(|computer| {
            !computer.is_local
                && Some(computer.id) != except
                && endpoints_match(&computer.ssh_destination, computer.ssh_port, ssh, port)
        })
    }

    fn local_computer_id(&self) -> Option<Uuid> {
        self.computers
            .iter()
            .find(|computer| computer.is_local)
            .map(|computer| computer.id)
    }

    fn select_fallback_host(&mut self) {
        let id = self
            .local_computer_id()
            .or_else(|| self.computers.first().map(|computer| computer.id));
        if let Some(id) = id {
            self.load_host_form(id);
        }
    }

    fn load_host_form(&mut self, id: Uuid) {
        let Some(computer) = self.computer(id).cloned() else {
            return;
        };
        let color = host_color(computer.id, computer.color.as_deref()).to_string();
        let port = match computer.ssh_port {
            Some(port) => port.to_string(),
            None => String::new(),
        };
        self.host_form = Some(HostForm {
            target: HostFormTarget::Existing(id),
            name: computer.name,
            ssh: computer.ssh_destination,
            port,
            color,
            confirm_delete: false,
            message: String::new(),
        });
    }

    pub fn apply_location_selection(&mut self, location_id: Option<Uuid>, side: Side) {
        let computer_id = location_id.and_then(|id| self.location(id).map(|l| l.computer_id));
        match side {
            Side::Source => {
                if location_id != self.source_location {
                    self.entries.clear();
                    self.selected.clear();
                    self.list_loaded_for = None;
                }
                self.source_location = location_id;
                if let Some(cid) = computer_id {
                    self.source_computer = Some(cid);
                }
                self.invalidate_preflight();
            }
            Side::Dest => {
                self.dest_location = location_id;
                if let Some(cid) = computer_id {
                    self.dest_computer = Some(cid);
                }
                self.invalidate_preflight();
            }
        }
    }

    pub fn select_location(&mut self, id: Uuid, side: Side) {
        if self.selections_locked() {
            return;
        }
        self.note_user_activity();
        self.apply_location_selection(Some(id), side);
        if side == Side::Source {
            self.ensure_files_listed();
        }
    }
}

fn host_record_dirty(computer: &Computer, form: &HostForm) -> bool {
    if form.name.trim() != computer.name {
        return true;
    }
    let color_pinned = computer.color.as_deref().is_some_and(|stored| {
        stored.eq_ignore_ascii_case(&form.color)
            && crate::util::HOST_PALETTE
                .iter()
                .any(|color| stored.eq_ignore_ascii_case(color))
    });
    if !color_pinned {
        return true;
    }
    if computer.is_local {
        return false;
    }
    if form.ssh.trim() != computer.ssh_destination {
        return true;
    }
    match parse_port_field(&form.port) {
        Ok(port) => port != computer.ssh_port,
        Err(()) => false,
    }
}

fn valid_new_folder_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Enter a folder name".into());
    }
    if name == "." || name == ".." {
        return Err("Invalid folder name".into());
    }
    if name.starts_with('.') {
        return Err("Hidden folder names are not used here".into());
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err("Folder name cannot contain a path".into());
    }
    Ok(())
}
