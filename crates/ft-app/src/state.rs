//! Toolkit-agnostic application state and background transfer jobs.

use crate::util::{folder_display_name, format_bytes, truncate_err};
use anyhow::Result;
use chrono::Utc;
use ft_exec::{self, DirEntry, HostRef, Progress, TransferPlan};
use ft_mdns::{DiscoveredHost, Discovery};
use ft_store::{Computer, Location, LocationKind, Store};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use uuid::Uuid;

pub enum BgMsg {
    ListResult(Result<Vec<DirEntry>, String>),
    BrowseList {
        path: PathBuf,
        result: Result<Vec<DirEntry>, String>,
    },
    Preflight(Result<String, String>),
    Progress(Progress),
    TransferDone(Result<(u64, bool), String>),
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavTab {
    Source,
    Files,
    Destination,
}

impl NavTab {
    pub fn prev(self) -> Option<Self> {
        match self {
            NavTab::Source => None,
            NavTab::Files => Some(NavTab::Source),
            NavTab::Destination => Some(NavTab::Files),
        }
    }

    pub fn next(self) -> Option<Self> {
        match self {
            NavTab::Source => Some(NavTab::Files),
            NavTab::Files => Some(NavTab::Destination),
            NavTab::Destination => None,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            NavTab::Source => "Source",
            NavTab::Files => "Files",
            NavTab::Destination => "Destination",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            NavTab::Source => "Choose a saved location, or add a new one.",
            NavTab::Files => "Select files and folders to copy.",
            NavTab::Destination => "Choose where the files should go.",
        }
    }
}

pub struct FolderBrowser {
    pub target: BrowseTarget,
    pub computer_id: Uuid,
    pub current_path: PathBuf,
    pub path_edit: String,
    pub entries: Vec<DirEntry>,
    pub loading: bool,
    pub error: Option<String>,
}

pub struct LocationPicker {
    pub side: Side,
    pub computer_id: Option<Uuid>,
    pub path_edit: String,
    pub show_manual_host: bool,
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

    pub preflight_ok: Option<Result<String, String>>,
    pub progress: Progress,
    pub transferring: bool,
    pub status_line: String,
    pub cancel: Arc<AtomicBool>,

    pub new_name: String,
    pub new_ssh: String,
    pub new_port: String,
    pub computer_msg: String,

    pub folder_browser: Option<FolderBrowser>,
    pub location_picker: Option<LocationPicker>,

    tx: Sender<BgMsg>,
    rx: Receiver<BgMsg>,
}

impl AppState {
    pub fn new() -> Result<Self> {
        let store = Store::open_default()?;
        let discovery = Discovery::start().ok();
        let (tx, rx) = mpsc::channel();
        let mut app = Self {
            store,
            discovery,
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
            preflight_ok: None,
            progress: Progress::default(),
            transferring: false,
            status_line: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            new_name: String::new(),
            new_ssh: String::new(),
            new_port: String::new(),
            computer_msg: String::new(),
            folder_browser: None,
            location_picker: None,
            tx,
            rx,
        };
        app.reload_store();
        if let Some(local) = app.computers.iter().find(|c| c.is_local) {
            app.source_computer = Some(local.id);
            app.dest_computer = Some(local.id);
        }
        Ok(app)
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
            .filter_map(|c| {
                let locs = by_host.get(&c.id).cloned().unwrap_or_default();
                if locs.is_empty() {
                    None
                } else {
                    Some((c.clone(), locs))
                }
            })
            .collect()
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

    fn pick_native_folder(&self, title: &str) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(home) = dirs::home_dir() {
            dialog = dialog.set_directory(home);
        }
        dialog.pick_folder()
    }

    pub fn open_folder_browse(&mut self, computer_id: Uuid, target: BrowseTarget) {
        let Some(c) = self.computer(computer_id).cloned() else {
            return;
        };
        if c.is_local {
            let title = match target {
                BrowseTarget::Source => "Choose source folder",
                BrowseTarget::Dest => "Choose destination folder",
            };
            if let Some(path) = self.pick_native_folder(title) {
                self.apply_browsed_folder(computer_id, target, path);
            }
            return;
        }

        let current = self
            .location_picker
            .as_ref()
            .map(|p| p.path_edit.trim())
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| match target {
                BrowseTarget::Source => self
                    .source_location
                    .and_then(|id| self.location(id))
                    .map(|l| l.path.clone()),
                BrowseTarget::Dest => self
                    .dest_location
                    .and_then(|id| self.location(id))
                    .map(|l| l.path.clone()),
            })
            .unwrap_or_else(|| PathBuf::from("~"));

        self.folder_browser = Some(FolderBrowser {
            target,
            computer_id,
            current_path: current.clone(),
            path_edit: current.to_string_lossy().into_owned(),
            entries: vec![],
            loading: true,
            error: None,
        });
        self.refresh_folder_browser();
    }

    pub fn apply_browsed_folder(&mut self, computer_id: Uuid, target: BrowseTarget, path: PathBuf) {
        match target {
            BrowseTarget::Source => {
                let id = self.ensure_location(computer_id, path);
                self.apply_location_selection(Some(id), Side::Source);
                self.location_picker = None;
                self.start_list();
            }
            BrowseTarget::Dest => {
                let id = self.ensure_location(computer_id, path);
                self.apply_location_selection(Some(id), Side::Dest);
                self.location_picker = None;
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
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let host = AppState::host_ref(&c);
            if path == PathBuf::from("~") {
                path = match ft_exec::remote_home(&host) {
                    Ok(h) => h,
                    Err(e) => {
                        let _ = tx.send(BgMsg::BrowseList {
                            path: PathBuf::from("~"),
                            result: Err(format!("{e:#}")),
                        });
                        return;
                    }
                };
            }
            let result = ft_exec::list_dir(&host, &path).map_err(|e| format!("{e:#}"));
            let _ = tx.send(BgMsg::BrowseList { path, result });
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

    pub fn browser_select(&mut self) {
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
                BgMsg::Preflight(r) => {
                    self.preflight_ok = Some(r);
                }
                BgMsg::Progress(p) => {
                    let data_complete = p.data_complete;
                    let bytes_done = p.bytes_done;
                    self.progress = p;
                    if data_complete {
                        self.mark_transfer_complete(bytes_done, false);
                    }
                }
                BgMsg::TransferDone(r) => match r {
                    Ok((bytes, cancelled)) => {
                        if self.transferring {
                            self.mark_transfer_complete(bytes, cancelled);
                        }
                    }
                    Err(e) => {
                        self.transferring = false;
                        self.status_line = format!("Transfer failed: {e}");
                    }
                },
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
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let host = AppState::host_ref(&c);
            let res = ft_exec::list_dir(&host, &loc.path).map_err(|e| format!("{e:#}"));
            let _ = tx.send(BgMsg::ListResult(res));
        });
    }

    pub fn refresh_file_list(&mut self) {
        self.list_loaded_for = None;
        self.start_list();
    }

    pub fn source_ready(&self) -> bool {
        self.source_location.is_some()
    }

    pub fn files_ready(&self) -> bool {
        !self.selected.is_empty()
    }

    pub fn transfer_ready(&self) -> bool {
        self.source_ready()
            && self.files_ready()
            && self.dest_location.is_some()
            && !self.transferring
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
        self.tab = tab;
        if tab == NavTab::Files {
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
            self.tab = prev;
        }
    }

    pub fn can_advance(&self) -> bool {
        match self.tab {
            NavTab::Source => self.source_ready(),
            NavTab::Files => self.files_ready(),
            NavTab::Destination => true,
        }
    }

    pub fn run_preflight(&mut self) {
        match self.build_plan() {
            Ok(plan) => {
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let res = ft_exec::preflight_start(&plan)
                        .map(|_| format!("OK — mode {:?}", plan.mode))
                        .map_err(|e| format!("{e:#}"));
                    let _ = tx.send(BgMsg::Preflight(res));
                });
            }
            Err(e) => self.preflight_ok = Some(Err(e)),
        }
    }

    fn build_plan(&self) -> Result<TransferPlan, String> {
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
        let relatives: Vec<String> = self.selected.iter().cloned().collect();
        ft_exec::plan_transfer(
            Self::host_ref(&sc),
            Self::host_ref(&dc),
            sl.path,
            dl.path,
            relatives,
        )
        .map_err(|e| format!("{e:#}"))
    }

    pub fn start_transfer(&mut self) {
        let plan = match self.build_plan() {
            Ok(p) => p,
            Err(e) => {
                self.status_line = e;
                return;
            }
        };
        if let Err(e) = ft_exec::preflight_start(&plan) {
            self.preflight_ok = Some(Err(format!("{e:#}")));
            self.status_line = format!("Cannot start: {e:#}");
            return;
        }

        self.transferring = true;
        self.cancel.store(false, Ordering::SeqCst);
        self.progress = Progress {
            bytes_total: plan.bytes_total,
            indeterminate: plan.bytes_total.is_none(),
            ..Default::default()
        };
        self.status_line = "Transferring…".into();

        let tx = self.tx.clone();
        let cancel = self.cancel.clone();

        std::thread::spawn(move || {
            let on_prog = {
                let tx = tx.clone();
                move |p: Progress| {
                    let _ = tx.send(BgMsg::Progress(p));
                }
            };
            let result = ft_exec::run_transfer(&plan, cancel, on_prog);
            match result {
                Ok(r) => {
                    let _ = tx.send(BgMsg::TransferDone(Ok((r.bytes_transferred, r.cancelled))));
                }
                Err(e) => {
                    let _ = tx.send(BgMsg::TransferDone(Err(truncate_err(&format!("{e:#}")))));
                }
            }
        });
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn mark_transfer_complete(&mut self, bytes: u64, cancelled: bool) {
        if !self.transferring {
            return;
        }
        self.transferring = false;
        self.status_line = if cancelled {
            "Cancelled".into()
        } else {
            format!("Transfer complete ({})", format_bytes(bytes))
        };
    }

    pub fn select_all_files(&mut self) {
        for e in &self.entries {
            self.selected.insert(e.name.clone());
        }
    }

    pub fn clear_file_selection(&mut self) {
        self.selected.clear();
    }

    pub fn toggle_file(&mut self, name: &str) {
        if self.selected.contains(name) {
            self.selected.remove(name);
        } else {
            self.selected.insert(name.to_string());
        }
    }

    pub fn delete_location_tile(&mut self, id: Uuid) {
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

    pub fn open_location_picker(&mut self, side: Side) {
        let computer_id = match side {
            Side::Source => self.source_computer,
            Side::Dest => self.dest_computer,
        }
        .or_else(|| self.computers.iter().find(|c| c.is_local).map(|c| c.id))
        .or_else(|| self.computers.first().map(|c| c.id));

        self.location_picker = Some(LocationPicker {
            side,
            computer_id,
            path_edit: String::new(),
            show_manual_host: false,
        });
        self.computer_msg.clear();
    }

    pub fn picker_set_computer(&mut self, id: Uuid) {
        if let Some(picker) = &mut self.location_picker {
            picker.computer_id = Some(id);
        }
    }

    pub fn add_discovered_host(&mut self, name: String, host: String, port: u16) {
        let now = Utc::now();
        let bonjour = name.clone();
        let c = Computer {
            id: Uuid::new_v4(),
            name: name.clone(),
            ssh_destination: host,
            ssh_port: Some(port).filter(|&p| p != 22),
            identity_file: None,
            bonjour_name: Some(bonjour),
            last_seen_at: Some(now),
            is_local: false,
            created_at: now,
            updated_at: now,
        };
        let _ = self.store.upsert_computer(&c);
        if let Some(picker) = &mut self.location_picker {
            picker.computer_id = Some(c.id);
        }
        self.reload_store();
        self.computer_msg = format!("Added host {name}");
    }

    pub fn add_manual_host(&mut self) {
        let now = Utc::now();
        let port = self.new_port.trim().parse().ok();
        let c = Computer {
            id: Uuid::new_v4(),
            name: self.new_name.trim().to_string(),
            ssh_destination: self.new_ssh.trim().to_string(),
            ssh_port: port,
            identity_file: None,
            bonjour_name: None,
            last_seen_at: None,
            is_local: false,
            created_at: now,
            updated_at: now,
        };
        if c.name.is_empty() || c.ssh_destination.is_empty() {
            self.computer_msg = "Name and SSH destination required".into();
        } else {
            let _ = self.store.upsert_computer(&c);
            if let Some(picker) = &mut self.location_picker {
                picker.computer_id = Some(c.id);
            }
            self.new_name.clear();
            self.new_ssh.clear();
            self.new_port.clear();
            self.reload_store();
            self.computer_msg = format!("Added host {}", c.name);
        }
    }

    pub fn picker_browse(&mut self) {
        let Some(picker) = &self.location_picker else {
            return;
        };
        let side = picker.side;
        let computer_id = picker.computer_id;
        if let Some(cid) = computer_id {
            let target = match side {
                Side::Source => BrowseTarget::Source,
                Side::Dest => BrowseTarget::Dest,
            };
            self.open_folder_browse(cid, target);
        } else {
            self.computer_msg = "Choose a host first".into();
        }
    }

    pub fn picker_use_path(&mut self) {
        let Some(picker) = &self.location_picker else {
            return;
        };
        let side = picker.side;
        let computer_id = picker.computer_id;
        let path = picker.path_edit.trim().to_string();
        if let Some(cid) = computer_id {
            if !path.is_empty() {
                let id = self.ensure_location(cid, PathBuf::from(path));
                self.apply_location_selection(Some(id), side);
                if side == Side::Source {
                    self.start_list();
                }
                self.location_picker = None;
            } else {
                self.computer_msg = "Path required".into();
            }
        } else {
            self.computer_msg = "Choose a host first".into();
        }
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
            }
            Side::Dest => {
                self.dest_location = location_id;
                if let Some(cid) = computer_id {
                    self.dest_computer = Some(cid);
                }
            }
        }
    }

    pub fn select_location(&mut self, id: Uuid, side: Side) {
        self.apply_location_selection(Some(id), side);
        if side == Side::Source {
            self.ensure_files_listed();
        }
    }
}
