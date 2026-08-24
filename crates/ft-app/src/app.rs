//! egui UI for File Transfer.app

use crate::icons::Icon;
use crate::theme::{self, colors};
use crate::util::{format_bytes, truncate_err};
use crate::widgets::{self, NavTab};
use anyhow::Result;
use chrono::Utc;
use eframe::egui;
use ft_exec::{self, DirEntry, HostRef, Progress, TransferPlan};
use ft_mdns::Discovery;
use ft_store::{Computer, JobRecord, JobStatus, Location, LocationKind, Store};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use uuid::Uuid;

enum BgMsg {
    ListResult(Result<Vec<DirEntry>, String>),
    BrowseList {
        path: PathBuf,
        result: Result<Vec<DirEntry>, String>,
    },
    Preflight(Result<String, String>),
    Progress(Progress),
    TransferDone(Result<(Uuid, u64, bool), String>),
    TestHost(Result<String, String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BrowseTarget {
    Source,
    Dest,
    LocationsForm,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Source,
    Dest,
}

struct FolderBrowser {
    target: BrowseTarget,
    computer_id: Uuid,
    current_path: PathBuf,
    path_edit: String,
    entries: Vec<DirEntry>,
    loading: bool,
    error: Option<String>,
}

pub struct FileTransferApp {
    store: Store,
    discovery: Option<Discovery>,
    tab: NavTab,

    computers: Vec<Computer>,
    locations: Vec<Location>,
    jobs: Vec<JobRecord>,

    source_computer: Option<Uuid>,
    source_location: Option<Uuid>,
    dest_computer: Option<Uuid>,
    dest_location: Option<Uuid>,

    entries: Vec<DirEntry>,
    selected: HashSet<String>,
    list_error: Option<String>,
    listing: bool,

    preflight_ok: Option<Result<String, String>>,
    progress: Progress,
    transferring: bool,
    active_job_id: Option<Uuid>,
    status_line: String,
    cancel: Arc<AtomicBool>,

    // Computers form
    new_name: String,
    new_ssh: String,
    new_port: String,
    computer_msg: String,

    // Locations form
    loc_computer: Option<Uuid>,
    loc_name: String,
    loc_path: String,
    location_msg: String,
    /// Inline path entry for source/dest.
    source_path_edit: String,
    dest_path_edit: String,

    folder_browser: Option<FolderBrowser>,

    tx: Sender<BgMsg>,
    rx: Receiver<BgMsg>,
}

impl FileTransferApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self> {
        theme::setup(cc);
        let store = Store::open_default()?;
        let discovery = Discovery::start().ok();
        let (tx, rx) = mpsc::channel();
        let mut app = Self {
            store,
            discovery,
            tab: NavTab::Transfer,
            computers: vec![],
            locations: vec![],
            jobs: vec![],
            source_computer: None,
            source_location: None,
            dest_computer: None,
            dest_location: None,
            entries: vec![],
            selected: HashSet::new(),
            list_error: None,
            listing: false,
            preflight_ok: None,
            progress: Progress::default(),
            transferring: false,
            active_job_id: None,
            status_line: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            new_name: String::new(),
            new_ssh: String::new(),
            new_port: String::new(),
            computer_msg: String::new(),
            loc_computer: None,
            loc_name: String::new(),
            loc_path: String::new(),
            location_msg: String::new(),
            source_path_edit: String::new(),
            dest_path_edit: String::new(),
            folder_browser: None,
            tx,
            rx,
        };
        app.reload_store();
        if let Some(local) = app.computers.iter().find(|c| c.is_local) {
            app.source_computer = Some(local.id);
            app.dest_computer = Some(local.id);
            app.loc_computer = Some(local.id);
        }
        Ok(app)
    }

    fn reload_store(&mut self) {
        self.computers = self.store.computers().unwrap_or_default();
        self.locations = self.store.locations().unwrap_or_default();
        self.jobs = self.store.jobs().unwrap_or_default();
    }

    fn computer(&self, id: Uuid) -> Option<&Computer> {
        self.computers.iter().find(|c| c.id == id)
    }

    fn location(&self, id: Uuid) -> Option<&Location> {
        self.locations.iter().find(|l| l.id == id)
    }

    /// Find or create a saved location for this computer + absolute path.
    fn ensure_location(&mut self, computer_id: Uuid, path: PathBuf) -> Uuid {
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
        let now = Utc::now();
        let loc = Location {
            id: Uuid::new_v4(),
            computer_id,
            name,
            path,
            kind: LocationKind::Either,
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

    /// Open folder picker: native for This Mac, SSH browser for remotes.
    fn open_folder_browse(&mut self, computer_id: Uuid, target: BrowseTarget) {
        let Some(c) = self.computer(computer_id).cloned() else {
            return;
        };
        if c.is_local {
            let title = match target {
                BrowseTarget::Source => "Choose source folder",
                BrowseTarget::Dest => "Choose destination folder",
                BrowseTarget::LocationsForm => "Choose folder to save",
            };
            if let Some(path) = self.pick_native_folder(title) {
                self.apply_browsed_folder(computer_id, target, path);
            }
            return;
        }

        let current = match target {
            BrowseTarget::Source => self
                .source_location
                .and_then(|id| self.location(id))
                .map(|l| l.path.clone()),
            BrowseTarget::Dest => self
                .dest_location
                .and_then(|id| self.location(id))
                .map(|l| l.path.clone()),
            BrowseTarget::LocationsForm => {
                let p = self.loc_path.trim();
                if p.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(p))
                }
            }
        }
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

    fn apply_browsed_folder(&mut self, computer_id: Uuid, target: BrowseTarget, path: PathBuf) {
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
            BrowseTarget::LocationsForm => {
                if self.loc_name.trim().is_empty() {
                    self.loc_name = path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                }
                self.loc_path = path.to_string_lossy().into_owned();
            }
        }
    }

    fn refresh_folder_browser(&mut self) {
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
            let host = FileTransferApp::host_ref(&c);
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

    fn host_ref(c: &Computer) -> HostRef {
        HostRef {
            is_local: c.is_local,
            ssh_destination: c.ssh_destination.clone(),
            ssh_port: c.ssh_port,
            identity_file: c.identity_file.clone(),
        }
    }

    fn poll_bg(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                BgMsg::ListResult(r) => {
                    self.listing = false;
                    match r {
                        Ok(e) => {
                            self.entries = e;
                            self.list_error = None;
                            self.selected.clear();
                        }
                        Err(e) => {
                            self.entries.clear();
                            self.list_error = Some(e);
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
                },
                BgMsg::TransferDone(r) => {
                    match r {
                        Ok((job_id, bytes, cancelled)) => {
                            if self.transferring || self.active_job_id == Some(job_id) {
                                self.mark_transfer_complete(bytes, cancelled);
                            } else if let Ok(mut jobs) = self.store.jobs() {
                                // Background thread finished after UI already marked complete.
                                if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
                                    if job.status == JobStatus::Running {
                                        job.finished_at = Some(Utc::now());
                                        job.bytes_transferred = Some(bytes);
                                        job.status = if cancelled {
                                            JobStatus::Cancelled
                                        } else {
                                            JobStatus::Ok
                                        };
                                        let _ = self.store.update_job(job);
                                        self.reload_store();
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            self.transferring = false;
                            self.active_job_id = None;
                            self.status_line = format!("Transfer failed: {e}");
                            self.reload_store();
                        }
                    }
                }
                BgMsg::TestHost(r) => match r {
                    Ok(m) => self.computer_msg = m,
                    Err(e) => self.computer_msg = format!("Failed: {e}"),
                },
            }
        }
    }

    fn start_list(&mut self) {
        let Some(cid) = self.source_computer else { return };
        let Some(lid) = self.source_location else { return };
        let Some(c) = self.computer(cid).cloned() else { return };
        let Some(loc) = self.location(lid).cloned() else { return };
        self.listing = true;
        self.list_error = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let host = FileTransferApp::host_ref(&c);
            let res = ft_exec::list_dir(&host, &loc.path).map_err(|e| format!("{e:#}"));
            let _ = tx.send(BgMsg::ListResult(res));
        });
    }

    fn run_preflight(&mut self) {
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

    fn start_transfer(&mut self) {
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

        let sc = self.source_computer.unwrap();
        let sl = self.source_location.unwrap();
        let dc = self.dest_computer.unwrap();
        let dl = self.dest_location.unwrap();
        let job = JobRecord {
            id: Uuid::new_v4(),
            started_at: Utc::now(),
            finished_at: None,
            source_computer_id: sc,
            source_location_id: sl,
            dest_computer_id: dc,
            dest_location_id: dl,
            bytes_total: plan.bytes_total,
            bytes_transferred: None,
            file_count: Some(plan.file_count),
            status: JobStatus::Running,
            error_summary: None,
        };
        let job_id = job.id;
        let _ = self.store.insert_job(&job);

        self.transferring = true;
        self.active_job_id = Some(job_id);
        self.cancel.store(false, Ordering::SeqCst);
        self.progress = Progress {
            bytes_total: plan.bytes_total,
            indeterminate: plan.bytes_total.is_none(),
            ..Default::default()
        };
        self.status_line = "Transferring…".into();

        let tx = self.tx.clone();
        let cancel = self.cancel.clone();
        let store_path_note = (); // privacy: no filenames to store
        let _ = store_path_note;

        // For failure updates we need store access — reopen in thread.
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
                    let _ = tx.send(BgMsg::TransferDone(Ok((
                        job_id,
                        r.bytes_transferred,
                        r.cancelled,
                    ))));
                }
                Err(e) => {
                    if let Ok(store) = Store::open_default() {
                        if let Ok(jobs) = store.jobs() {
                            if let Some(mut job) = jobs.into_iter().find(|j| j.id == job_id) {
                                job.finished_at = Some(Utc::now());
                                job.status = JobStatus::Failed;
                                job.error_summary = Some(truncate_err(&format!("{e:#}")));
                                let _ = store.update_job(&job);
                            }
                        }
                    }
                    let _ = tx.send(BgMsg::TransferDone(Err(truncate_err(&format!("{e:#}")))));
                }
            }
        });
    }

    fn mark_transfer_complete(&mut self, bytes: u64, cancelled: bool) {
        if !self.transferring && self.active_job_id.is_none() {
            return;
        }
        self.transferring = false;
        if let Some(job_id) = self.active_job_id.take() {
            if let Ok(mut jobs) = self.store.jobs() {
                if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
                    job.finished_at = Some(Utc::now());
                    job.bytes_transferred = Some(bytes);
                    job.status = if cancelled {
                        JobStatus::Cancelled
                    } else {
                        JobStatus::Ok
                    };
                    let _ = self.store.update_job(job);
                }
            }
        }
        self.status_line = if cancelled {
            "Cancelled".into()
        } else {
            format!("Transfer complete ({})", format_bytes(bytes))
        };
        self.reload_store();
    }
}

impl eframe::App for FileTransferApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_bg();
        let interval = if self.transferring { 100 } else { 250 };
        ctx.request_repaint_after(std::time::Duration::from_millis(interval));

        egui::TopBottomPanel::bottom("progress_footer")
            .exact_height(72.0)
            .frame(theme::footer_frame())
            .show(ctx, |ui| {
                let transferring = self.transferring;
                let cancel = self.cancel.clone();
                widgets::progress_footer(
                    ui,
                    &self.progress,
                    transferring,
                    &self.status_line,
                    move || cancel.store(true, Ordering::SeqCst),
                );
            });

        egui::SidePanel::left("sidebar")
            .exact_width(220.0)
            .resizable(false)
            .frame(theme::sidebar_frame())
            .show(ctx, |ui| {
                widgets::app_sidebar(ui, &mut self.tab);
            });

        egui::CentralPanel::default()
            .frame(theme::content_frame())
            .show(ctx, |ui| match self.tab {
                NavTab::Transfer => self.ui_transfer(ui),
                NavTab::Computers => self.ui_computers(ui),
                NavTab::Locations => self.ui_locations(ui),
                NavTab::History => self.ui_history(ui),
            });

        self.ui_folder_browser(ctx);
    }
}

impl FileTransferApp {
    fn ui_folder_browser(&mut self, ctx: &egui::Context) {
        let Some(browser) = &self.folder_browser else {
            return;
        };
        let title = match browser.target {
            BrowseTarget::Source => "Browse source folder",
            BrowseTarget::Dest => "Browse destination folder",
            BrowseTarget::LocationsForm => "Browse folder",
        };
        let computer_name = self
            .computer(browser.computer_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "remote".into());

        let mut open = true;
        let mut close = false;
        let mut select = false;
        let mut go_up = false;
        let mut go_home = false;
        let mut go_path: Option<PathBuf> = None;
        let mut enter: Option<String> = None;
        let mut refresh = false;

        egui::Window::new(format!("{title} — {computer_name}"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([540.0, 460.0])
            .frame(theme::card_frame())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    widgets::field_label(ui, "Path");
                    if let Some(browser) = &mut self.folder_browser {
                        ui.add(
                            egui::TextEdit::singleline(&mut browser.path_edit)
                                .desired_width(ui.available_width() - 60.0),
                        );
                        if widgets::secondary_button(ui, "Go").clicked() {
                            let p = browser.path_edit.trim();
                            if !p.is_empty() {
                                go_path = Some(PathBuf::from(p));
                            }
                        }
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if widgets::icon_button(ui, Icon::Home, "Home").clicked() {
                        go_home = true;
                    }
                    if widgets::secondary_button(ui, "Up").clicked() {
                        go_up = true;
                    }
                    if widgets::icon_button(ui, Icon::Refresh, "Refresh").clicked() {
                        refresh = true;
                    }
                    if let Some(browser) = &self.folder_browser {
                        if browser.loading {
                            ui.spinner();
                        }
                    }
                });

                if let Some(browser) = &self.folder_browser {
                    if let Some(err) = &browser.error {
                        ui.horizontal(|ui| {
                            Icon::Xmark.ui(ui, colors::ERROR);
                            ui.colored_label(colors::ERROR, err);
                        });
                    }
                    ui.label(
                        egui::RichText::new(format!("{}", browser.current_path.display()))
                            .size(12.0)
                            .color(colors::TEXT_SECONDARY),
                    );
                }

                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .id_salt("browse_dirs")
                    .max_height(300.0)
                    .show(ui, |ui| {
                        let entries = self
                            .folder_browser
                            .as_ref()
                            .map(|b| b.entries.clone())
                            .unwrap_or_default();
                        for e in entries {
                            ui.horizontal(|ui| {
                                Icon::Folder.ui(ui, colors::ACCENT);
                                if ui
                                    .selectable_label(false, format!("{}/", e.name))
                                    .double_clicked()
                                {
                                    enter = Some(e.name.clone());
                                }
                                if widgets::secondary_button(ui, "Open").clicked() {
                                    enter = Some(e.name.clone());
                                }
                            });
                        }
                        let empty = self
                            .folder_browser
                            .as_ref()
                            .map(|b| b.entries.is_empty() && !b.loading)
                            .unwrap_or(false);
                        if empty {
                            ui.label(
                                egui::RichText::new("No subfolders")
                                    .color(colors::TEXT_SECONDARY),
                            );
                        }
                    });

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if widgets::primary_button(ui, "Select Folder").clicked() {
                        select = true;
                    }
                    if widgets::secondary_button(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            });

        if !open {
            close = true;
        }

        if let Some(path) = go_path {
            if let Some(browser) = &mut self.folder_browser {
                browser.current_path = path;
                browser.loading = true;
            }
            self.refresh_folder_browser();
        } else if go_up {
            if let Some(browser) = &mut self.folder_browser {
                if let Some(parent) = browser.current_path.parent() {
                    browser.current_path = parent.to_path_buf();
                    if browser.current_path.as_os_str().is_empty() {
                        browser.current_path = PathBuf::from("/");
                    }
                }
            }
            self.refresh_folder_browser();
        } else if go_home {
            if let Some(browser) = &mut self.folder_browser {
                browser.current_path = PathBuf::from("~");
            }
            self.refresh_folder_browser();
        } else if let Some(name) = enter {
            if let Some(browser) = &mut self.folder_browser {
                browser.current_path = browser.current_path.join(name);
            }
            self.refresh_folder_browser();
        } else if refresh {
            self.refresh_folder_browser();
        } else if select {
            if let Some(browser) = self.folder_browser.take() {
                self.apply_browsed_folder(
                    browser.computer_id,
                    browser.target,
                    browser.current_path,
                );
            }
        } else if close {
            self.folder_browser = None;
        }
    }

    fn ui_transfer(&mut self, ui: &mut egui::Ui) {
        theme::section_heading(
            ui,
            "Transfer",
            Some("Select source and destination, then choose files to copy."),
        );
        ui.add_space(16.0);

        let mut list_after_pick = false;

        theme::card_section(ui, "Source", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.vertical(|ui| {
                    widgets::field_label(ui, "Saved location");
                    let mut src_l = self.source_location;
                    let changed = Self::location_combo_ui(
                        ui,
                        &self.computers,
                        &self.locations,
                        &mut src_l,
                        "src_loc",
                    );
                    if changed {
                        self.apply_location_selection(src_l, Side::Source);
                        list_after_pick = self.source_location.is_some();
                    }
                });

                ui.add_space(16.0);

                ui.vertical(|ui| {
                    widgets::field_label(ui, "Browse on");
                    ui.horizontal(|ui| {
                        let mut src_c = self.source_computer;
                        Self::computer_combo_ui(ui, &self.computers, &mut src_c, "src_browse_host");
                        if src_c != self.source_computer {
                            self.source_computer = src_c;
                        }
                    });
                });

                ui.add_space(8.0);

                ui.vertical(|ui| {
                    widgets::field_label(ui, " ");
                    ui.horizontal(|ui| {
                        if widgets::icon_button(ui, Icon::FolderOpen, "Browse…").clicked() {
                            if let Some(cid) = self.source_computer {
                                self.open_folder_browse(cid, BrowseTarget::Source);
                            }
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.source_path_edit)
                                .desired_width(200.0)
                                .hint_text("Path…"),
                        );
                        if widgets::secondary_button(ui, "Use").clicked() {
                            let p = self.source_path_edit.trim();
                            if !p.is_empty() {
                                if let Some(cid) = self.source_computer {
                                    let id = self.ensure_location(cid, PathBuf::from(p));
                                    self.apply_location_selection(Some(id), Side::Source);
                                    list_after_pick = true;
                                }
                            }
                        }
                        if widgets::secondary_button(ui, "List files").clicked() {
                            list_after_pick = true;
                        }
                        if self.listing {
                            ui.spinner();
                        }
                    });
                });
            });

            if let Some(err) = &self.list_error {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    Icon::Xmark.ui(ui, colors::ERROR);
                    ui.colored_label(colors::ERROR, err);
                });
            }
        });

        ui.add_space(12.0);

        theme::card_section(ui, "Files", |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{} selected", self.selected.len()))
                        .color(colors::TEXT_SECONDARY),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::secondary_button(ui, "Clear").clicked() {
                        self.selected.clear();
                    }
                    if widgets::secondary_button(ui, "Select all").clicked() {
                        for e in &self.entries {
                            self.selected.insert(e.name.clone());
                        }
                    }
                });
            });
            ui.add_space(8.0);

            egui::ScrollArea::vertical()
                .id_salt("file_list")
                .max_height(240.0)
                .show(ui, |ui| {
                    if self.entries.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(24.0);
                            Icon::Document.ui(ui, colors::TEXT_SECONDARY);
                            ui.label(
                                egui::RichText::new("Choose a source and list files")
                                    .color(colors::TEXT_SECONDARY),
                            );
                        });
                    } else {
                        for e in &self.entries {
                            let mut on = self.selected.contains(&e.name);
                            ui.horizontal(|ui| {
                                let icon = if e.is_dir {
                                    Icon::Folder
                                } else {
                                    Icon::Document
                                };
                                icon.ui(ui, colors::TEXT_SECONDARY);
                                let label = if e.is_dir {
                                    format!("{}/", e.name)
                                } else {
                                    format!("{}  ·  {}", e.name, format_bytes(e.size))
                                };
                                if ui.checkbox(&mut on, label).changed() {
                                    if on {
                                        self.selected.insert(e.name.clone());
                                    } else {
                                        self.selected.remove(&e.name);
                                    }
                                }
                            });
                        }
                    }
                });
        });

        ui.add_space(12.0);

        theme::card_section(ui, "Destination", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.vertical(|ui| {
                    widgets::field_label(ui, "Saved location");
                    let mut dst_l = self.dest_location;
                    let changed = Self::location_combo_ui(
                        ui,
                        &self.computers,
                        &self.locations,
                        &mut dst_l,
                        "dst_loc",
                    );
                    if changed {
                        self.apply_location_selection(dst_l, Side::Dest);
                    }
                });

                ui.add_space(16.0);

                ui.vertical(|ui| {
                    widgets::field_label(ui, "Browse on");
                    ui.horizontal(|ui| {
                        let mut dst_c = self.dest_computer;
                        Self::computer_combo_ui(ui, &self.computers, &mut dst_c, "dst_browse_host");
                        if dst_c != self.dest_computer {
                            self.dest_computer = dst_c;
                        }
                    });
                });

                ui.add_space(8.0);

                ui.vertical(|ui| {
                    widgets::field_label(ui, " ");
                    ui.horizontal(|ui| {
                        if widgets::icon_button(ui, Icon::FolderOpen, "Browse…").clicked() {
                            if let Some(cid) = self.dest_computer {
                                self.open_folder_browse(cid, BrowseTarget::Dest);
                            }
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.dest_path_edit)
                                .desired_width(200.0)
                                .hint_text("Path…"),
                        );
                        if widgets::secondary_button(ui, "Use").clicked() {
                            let p = self.dest_path_edit.trim();
                            if !p.is_empty() {
                                if let Some(cid) = self.dest_computer {
                                    let id = self.ensure_location(cid, PathBuf::from(p));
                                    self.apply_location_selection(Some(id), Side::Dest);
                                }
                            }
                        }
                    });
                });
            });
        });

        if list_after_pick {
            self.start_list();
        }

        ui.add_space(16.0);

        ui.horizontal(|ui| {
            if widgets::secondary_button(ui, "Check access").clicked() {
                self.run_preflight();
            }
            if let Some(preflight) = &self.preflight_ok {
                widgets::status_message(ui, preflight);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_start = !self.transferring
                    && self.source_computer.is_some()
                    && self.source_location.is_some()
                    && self.dest_computer.is_some()
                    && self.dest_location.is_some()
                    && !self.selected.is_empty();
                ui.add_enabled_ui(can_start, |ui| {
                    if widgets::primary_button(ui, "Start Transfer").clicked() {
                        self.start_transfer();
                    }
                });
            });
        });
    }

    fn apply_location_selection(&mut self, location_id: Option<Uuid>, side: Side) {
        let computer_id = location_id.and_then(|id| self.location(id).map(|l| l.computer_id));
        match side {
            Side::Source => {
                if location_id != self.source_location {
                    self.entries.clear();
                    self.selected.clear();
                    self.source_path_edit.clear();
                }
                self.source_location = location_id;
                if let Some(cid) = computer_id {
                    self.source_computer = Some(cid);
                }
            }
            Side::Dest => {
                if location_id != self.dest_location {
                    self.dest_path_edit.clear();
                }
                self.dest_location = location_id;
                if let Some(cid) = computer_id {
                    self.dest_computer = Some(cid);
                }
            }
        }
    }

    fn location_label(computers: &[Computer], loc: &Location) -> String {
        let host = computers
            .iter()
            .find(|c| c.id == loc.computer_id)
            .map(|c| c.name.as_str())
            .unwrap_or("?");
        format!("{host} — {} ({})", loc.name, loc.path.display())
    }

    /// Returns true when the selected location id changed.
    fn location_combo_ui(
        ui: &mut egui::Ui,
        computers: &[Computer],
        locations: &[Location],
        selected: &mut Option<Uuid>,
        id: &str,
    ) -> bool {
        let before = *selected;
        let selected_text = selected
            .and_then(|lid| locations.iter().find(|l| l.id == lid))
            .map(|l| Self::location_label(computers, l))
            .unwrap_or_else(|| "(choose host / folder)".into());
        egui::ComboBox::from_id_salt(id)
            .width(360.0)
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                if locations.is_empty() {
                    ui.label("No saved locations yet — browse or add on Locations tab");
                }
                for l in locations {
                    let label = Self::location_label(computers, l);
                    ui.selectable_value(selected, Some(l.id), label);
                }
            });
        *selected != before
    }

    fn computer_combo_ui(
        ui: &mut egui::Ui,
        computers: &[Computer],
        selected: &mut Option<Uuid>,
        id: &str,
    ) {
        let selected_text = selected
            .and_then(|id| computers.iter().find(|c| c.id == id))
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "(choose)".into());
        egui::ComboBox::from_id_salt(id)
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                for c in computers {
                    ui.selectable_value(selected, Some(c.id), &c.name);
                }
            });
    }

    fn ui_computers(&mut self, ui: &mut egui::Ui) {
        theme::section_heading(
            ui,
            "Computers",
            Some("Saved SSH hosts and Bonjour discoveries on the network."),
        );
        ui.add_space(16.0);

        let discovered = self
            .discovery
            .as_ref()
            .map(|d| d.hosts())
            .unwrap_or_default();

        if !discovered.is_empty() {
            theme::card_section(ui, "Discovered on network", |ui| {
                ui.horizontal(|ui| {
                    Icon::Network.ui(ui, colors::ACCENT);
                    ui.label(
                        egui::RichText::new("_ssh._tcp")
                            .size(12.0)
                            .color(colors::TEXT_SECONDARY),
                    );
                });
                ui.add_space(8.0);
                for h in &discovered {
                    ui.horizontal(|ui| {
                        Icon::Computer.ui(ui, colors::TEXT_SECONDARY);
                        ui.label(format!("{} — {}:{}", h.name, h.host, h.port));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::secondary_button(ui, "Save").clicked() {
                                let now = Utc::now();
                                let c = Computer {
                                    id: Uuid::new_v4(),
                                    name: h.name.clone(),
                                    ssh_destination: h.host.clone(),
                                    ssh_port: Some(h.port).filter(|&p| p != 22),
                                    identity_file: None,
                                    bonjour_name: Some(h.name.clone()),
                                    last_seen_at: Some(now),
                                    is_local: false,
                                    created_at: now,
                                    updated_at: now,
                                };
                                let _ = self.store.upsert_computer(&c);
                                self.reload_store();
                                self.computer_msg = format!("Saved {}", c.name);
                            }
                        });
                    });
                    ui.add_space(4.0);
                }
            });
            ui.add_space(12.0);
        }

        theme::card_section(ui, "Saved computers", |ui| {
            let mut delete = None;
            let mut test = None;
            for c in &self.computers {
                ui.horizontal(|ui| {
                    Icon::Computer.ui(
                        ui,
                        if c.is_local {
                            colors::ACCENT
                        } else {
                            colors::TEXT_SECONDARY
                        },
                    );
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(&c.name)
                                .strong()
                                .color(colors::TEXT_PRIMARY),
                        );
                        if c.is_local {
                            ui.label(
                                egui::RichText::new("This Mac")
                                    .size(12.0)
                                    .color(colors::TEXT_SECONDARY),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(&c.ssh_destination)
                                    .size(12.0)
                                    .color(colors::TEXT_SECONDARY),
                            );
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !c.is_local && widgets::secondary_button(ui, "Delete").clicked() {
                            delete = Some(c.id);
                        }
                        if !c.is_local && widgets::secondary_button(ui, "Test SSH").clicked() {
                            test = Some(c.clone());
                        }
                    });
                });
                ui.add_space(8.0);
            }
            if let Some(id) = delete {
                let _ = self.store.delete_computer(id);
                self.reload_store();
            }
            if let Some(c) = test {
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let host = FileTransferApp::host_ref(&c);
                    let res = ft_exec::test_ssh(&host)
                        .map(|_| format!("SSH OK: {}", c.ssh_destination))
                        .map_err(|e| format!("{e:#}"));
                    let _ = tx.send(BgMsg::TestHost(res));
                });
            }
        });

        ui.add_space(12.0);

        theme::card_section(ui, "Add computer", |ui| {
            egui::Grid::new("comp_add")
                .num_columns(2)
                .spacing([12.0, 10.0])
                .show(ui, |ui| {
                    widgets::field_label(ui, "Display name");
                    ui.text_edit_singleline(&mut self.new_name);
                    ui.end_row();
                    widgets::field_label(ui, "SSH destination");
                    ui.text_edit_singleline(&mut self.new_ssh);
                    ui.end_row();
                    widgets::field_label(ui, "Port (optional)");
                    ui.text_edit_singleline(&mut self.new_port);
                    ui.end_row();
                });
            ui.add_space(10.0);
            if widgets::primary_button(ui, "Add Computer").clicked() {
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
                    self.new_name.clear();
                    self.new_ssh.clear();
                    self.new_port.clear();
                    self.reload_store();
                    self.computer_msg = "Added".into();
                }
            }
        });

        if !self.computer_msg.is_empty() {
            ui.add_space(8.0);
            ui.label(&self.computer_msg);
        }
    }

    fn ui_locations(&mut self, ui: &mut egui::Ui) {
        theme::section_heading(
            ui,
            "Locations",
            Some("Saved host and folder combinations for quick selection."),
        );
        ui.add_space(16.0);

        theme::card_section(ui, "Saved locations", |ui| {
            let mut delete = None;
            if self.locations.is_empty() {
                ui.label(
                    egui::RichText::new("No locations saved yet.")
                        .color(colors::TEXT_SECONDARY),
                );
            }
            for loc in &self.locations {
                let cname = self
                    .computer(loc.computer_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("?");
                ui.horizontal(|ui| {
                    Icon::Folder.ui(ui, colors::ACCENT);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{cname} — {}", loc.name))
                                .strong()
                                .color(colors::TEXT_PRIMARY),
                        );
                        ui.label(
                            egui::RichText::new(loc.path.to_string_lossy())
                                .size(12.0)
                                .color(colors::TEXT_SECONDARY),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::secondary_button(ui, "Delete").clicked() {
                            delete = Some(loc.id);
                        }
                    });
                });
                ui.add_space(8.0);
            }
            if let Some(id) = delete {
                let _ = self.store.delete_location(id);
                self.reload_store();
            }
        });

        ui.add_space(12.0);

        theme::card_section(ui, "Add location", |ui| {
            egui::Grid::new("loc_add")
                .num_columns(2)
                .spacing([12.0, 10.0])
                .show(ui, |ui| {
                    widgets::field_label(ui, "Computer");
                    let text = self
                        .loc_computer
                        .and_then(|id| self.computer(id))
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| "(choose)".into());
                    egui::ComboBox::from_id_salt("add_loc_comp")
                        .selected_text(text)
                        .width(280.0)
                        .show_ui(ui, |ui| {
                            for c in &self.computers {
                                ui.selectable_value(&mut self.loc_computer, Some(c.id), &c.name);
                            }
                        });
                    ui.end_row();
                    widgets::field_label(ui, "Name");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.loc_name)
                            .hint_text("Optional — defaults to folder name"),
                    );
                    ui.end_row();
                    widgets::field_label(ui, "Path");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.loc_path)
                                .desired_width(240.0)
                                .hint_text("absolute path"),
                        );
                        if widgets::icon_button(ui, Icon::FolderOpen, "Browse…").clicked() {
                            if let Some(cid) = self.loc_computer {
                                self.open_folder_browse(cid, BrowseTarget::LocationsForm);
                            } else {
                                self.location_msg = "Choose a computer first".into();
                            }
                        }
                    });
                    ui.end_row();
                });
            ui.add_space(10.0);
            if widgets::primary_button(ui, "Add Location").clicked() {
                if let Some(cid) = self.loc_computer {
                    let path = PathBuf::from(self.loc_path.trim());
                    if self.loc_path.trim().is_empty() {
                        self.location_msg = "Path required".into();
                    } else {
                        let name = if self.loc_name.trim().is_empty() {
                            path.file_name()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.to_string_lossy().into_owned())
                        } else {
                            self.loc_name.trim().to_string()
                        };
                        let now = Utc::now();
                        let loc = Location {
                            id: Uuid::new_v4(),
                            computer_id: cid,
                            name,
                            path,
                            kind: LocationKind::Either,
                            created_at: now,
                            updated_at: now,
                        };
                        let _ = self.store.upsert_location(&loc);
                        self.loc_name.clear();
                        self.loc_path.clear();
                        self.reload_store();
                        self.location_msg = "Added".into();
                    }
                } else {
                    self.location_msg = "Choose a computer".into();
                }
            }
        });

        if !self.location_msg.is_empty() {
            ui.add_space(8.0);
            ui.label(&self.location_msg);
        }
    }

    fn ui_history(&mut self, ui: &mut egui::Ui) {
        theme::section_heading(
            ui,
            "History",
            Some("Job metadata only — transferred filenames are never stored."),
        );
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            if widgets::secondary_button(ui, "Refresh").clicked() {
                self.reload_store();
            }
        });
        ui.add_space(8.0);

        theme::card_frame().show(ui, |ui| {
            if self.jobs.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(32.0);
                    Icon::History.ui(ui, colors::TEXT_SECONDARY);
                    ui.label(
                        egui::RichText::new("No transfers yet")
                            .color(colors::TEXT_SECONDARY),
                    );
                });
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for j in &self.jobs {
                        let src = self
                            .computer(j.source_computer_id)
                            .map(|c| c.name.as_str())
                            .unwrap_or("?");
                        let dst = self
                            .computer(j.dest_computer_id)
                            .map(|c| c.name.as_str())
                            .unwrap_or("?");
                        let sl = self
                            .location(j.source_location_id)
                            .map(|l| l.name.as_str())
                            .unwrap_or("?");
                        let dl = self
                            .location(j.dest_location_id)
                            .map(|l| l.name.as_str())
                            .unwrap_or("?");

                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(format!("{src}/{sl}"))
                                            .strong()
                                            .color(colors::TEXT_PRIMARY),
                                    );
                                    Icon::ArrowRight.ui(ui, colors::TEXT_SECONDARY);
                                    ui.label(
                                        egui::RichText::new(format!("{dst}/{dl}"))
                                            .strong()
                                            .color(colors::TEXT_PRIMARY),
                                    );
                                });
                                ui.label(
                                    egui::RichText::new(
                                        j.started_at.format("%Y-%m-%d %H:%M").to_string(),
                                    )
                                    .size(12.0)
                                    .color(colors::TEXT_SECONDARY),
                                );
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let st = match j.status {
                                    JobStatus::Ok => "OK",
                                    JobStatus::Failed => "Failed",
                                    JobStatus::Cancelled => "Cancelled",
                                    JobStatus::Running => "Running",
                                };
                                widgets::status_badge(ui, st);
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{}  ·  {} files",
                                        j.bytes_transferred
                                            .map(format_bytes)
                                            .unwrap_or_else(|| "—".into()),
                                        j.file_count
                                            .map(|n| n.to_string())
                                            .unwrap_or_else(|| "—".into()),
                                    ))
                                    .size(12.0)
                                    .color(colors::TEXT_SECONDARY),
                                );
                            });
                        });
                        if let Some(err) = &j.error_summary {
                            ui.horizontal(|ui| {
                                Icon::Xmark.ui(ui, colors::ERROR);
                                ui.colored_label(colors::ERROR, err);
                            });
                        }
                        ui.add_space(12.0);
                        ui.separator();
                        ui.add_space(8.0);
                    }
                });
            }
        });
    }
}
