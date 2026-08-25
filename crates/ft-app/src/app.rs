//! egui UI for File Transfer.app

use crate::icons::Icon;
use crate::location_tile::{LocationDragPayload, location_tile, location_tile_drop_tail};
use crate::theme::{self, colors};
use crate::util::{folder_display_name, format_bytes, host_color, truncate_err};
use crate::widgets::{self, NavTab, WizardNavAction};
use anyhow::Result;
use chrono::Utc;
use eframe::egui;
use ft_exec::{self, DirEntry, HostRef, Progress, TransferPlan};
use ft_mdns::Discovery;
use ft_store::{Computer, JobRecord, JobStatus, Location, LocationKind, Store};
use std::collections::{HashMap, HashSet};
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
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BrowseTarget {
    Source,
    Dest,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
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

struct LocationPicker {
    side: Side,
    computer_id: Option<Uuid>,
    path_edit: String,
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
    /// Source location id that `entries` currently reflect (`None` = stale / not loaded).
    list_loaded_for: Option<Uuid>,

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

    folder_browser: Option<FolderBrowser>,
    location_picker: Option<LocationPicker>,

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
            tab: NavTab::Source,
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
            list_loaded_for: None,
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

    /// Open folder picker: native for This Mac, SSH browser for remotes.
    fn open_folder_browse(&mut self, computer_id: Uuid, target: BrowseTarget) {
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

    fn apply_browsed_folder(&mut self, computer_id: Uuid, target: BrowseTarget, path: PathBuf) {
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
                            self.list_loaded_for = self.source_location;
                        }
                        Err(e) => {
                            self.entries.clear();
                            self.list_error = Some(e);
                            // Don't auto-retry; user can press Refresh.
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
        self.list_loaded_for = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let host = FileTransferApp::host_ref(&c);
            let res = ft_exec::list_dir(&host, &loc.path).map_err(|e| format!("{e:#}"));
            let _ = tx.send(BgMsg::ListResult(res));
        });
    }

    fn refresh_file_list(&mut self) {
        self.list_loaded_for = None;
        self.start_list();
    }

    fn source_ready(&self) -> bool {
        self.source_location.is_some()
    }

    fn files_ready(&self) -> bool {
        !self.selected.is_empty()
    }

    fn transfer_ready(&self) -> bool {
        self.source_ready()
            && self.files_ready()
            && self.dest_location.is_some()
            && !self.transferring
    }

    fn ensure_files_listed(&mut self) {
        let Some(lid) = self.source_location else {
            return;
        };
        if self.listing || self.list_loaded_for == Some(lid) {
            return;
        }
        self.start_list();
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
            .min_height(72.0)
            .frame(theme::footer_frame())
            .show(ctx, |ui| {
                let transferring = self.transferring;
                let can_transfer = self.transfer_ready();
                let cancel = self.cancel.clone();
                let start = widgets::progress_footer(
                    ui,
                    &self.progress,
                    transferring,
                    &self.status_line,
                    can_transfer,
                    move || cancel.store(true, Ordering::SeqCst),
                );
                if start {
                    self.start_transfer();
                }
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
            .show(ctx, |ui| {
                if self.tab.is_wizard() {
                    widgets::constrain_content(ui);
                    ui.vertical(|ui| {
                        widgets::constrain_content(ui);
                        const NAV_RESERVE: f32 = 68.0;
                        let content_h = (ui.available_height() - NAV_RESERVE).max(120.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), content_h),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| {
                                widgets::constrain_content(ui);
                                let scroll = matches!(
                                    self.tab,
                                    NavTab::Source | NavTab::Destination
                                );
                                if scroll {
                                    let salt = match self.tab {
                                        NavTab::Source => "source_scroll",
                                        NavTab::Destination => "dest_scroll",
                                        _ => "wizard_scroll",
                                    };
                                    egui::ScrollArea::vertical()
                                        .id_salt(salt)
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            widgets::constrain_content(ui);
                                            self.ui_wizard_step(ui);
                                        });
                                } else {
                                    self.ui_wizard_step(ui);
                                }
                            },
                        );
                        let can_advance = match self.tab {
                            NavTab::Source => self.source_ready(),
                            NavTab::Files => self.files_ready(),
                            NavTab::Destination => true,
                            NavTab::History => false,
                        };
                        match widgets::wizard_nav_bar(ui, self.tab, can_advance) {
                            WizardNavAction::Back => {
                                if let Some(prev) = self.tab.prev() {
                                    self.tab = prev;
                                }
                            }
                            WizardNavAction::Next => {
                                if let Some(next) = self.tab.next() {
                                    self.tab = next;
                                    if self.tab == NavTab::Files {
                                        self.ensure_files_listed();
                                    }
                                }
                            }
                            WizardNavAction::None => {}
                        }
                    });
                } else {
                    widgets::page_body(ui, |ui| self.ui_history(ui));
                }
            });

        self.ui_folder_browser(ctx);
        self.ui_location_picker(ctx);
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
                widgets::constrain_content(ui);
                ui.horizontal_wrapped(|ui| {
                    widgets::constrain_content(ui);
                    widgets::field_label(ui, "Path");
                    if let Some(browser) = &mut self.folder_browser {
                        widgets::path_field(ui, &mut browser.path_edit, "");
                        if widgets::secondary_button(ui, "Go").clicked() {
                            let p = browser.path_edit.trim();
                            if !p.is_empty() {
                                go_path = Some(PathBuf::from(p));
                            }
                        }
                    }
                });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
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
                            ui.spacing_mut().item_spacing.x = 6.0;
                            Icon::Xmark.ui(ui, colors::ERROR);
                            ui.add(
                                egui::Label::new(egui::RichText::new(err).color(colors::ERROR))
                                    .wrap(),
                            );
                        });
                    }
                    widgets::wrapped_label(
                        ui,
                        egui::RichText::new(format!("{}", browser.current_path.display()))
                            .size(12.0)
                            .color(colors::TEXT_SECONDARY),
                    );
                }

                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .id_salt("browse_dirs")
                    .auto_shrink([false, false])
                    .max_height(300.0)
                    .show(ui, |ui| {
                        widgets::constrain_content(ui);
                        let entries = self
                            .folder_browser
                            .as_ref()
                            .map(|b| b.entries.clone())
                            .unwrap_or_default();
                        for e in entries {
                            ui.horizontal_wrapped(|ui| {
                                widgets::constrain_content(ui);
                                Icon::Folder.ui(ui, colors::ACCENT);
                                let entry = ui.add(egui::SelectableLabel::new(
                                    false,
                                    format!("{}/", e.name),
                                ));
                                if entry.double_clicked()
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
                ui.horizontal_wrapped(|ui| {
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

    fn ui_wizard_step(&mut self, ui: &mut egui::Ui) {
        match self.tab {
            NavTab::Source => self.ui_step_source(ui),
            NavTab::Files => {
                self.ensure_files_listed();
                self.ui_step_files(ui);
            }
            NavTab::Destination => self.ui_step_destination(ui),
            NavTab::History => {}
        }
    }

    fn ui_step_source(&mut self, ui: &mut egui::Ui) {
        if theme::section_heading_with_action(
            ui,
            "Source",
            Some("Choose a saved location, or add a new one."),
            "Add location",
        ) {
            self.open_location_picker(Side::Source);
        }
        ui.add_space(12.0);
        self.ui_location_tiles(ui, Side::Source);
    }

    fn ui_step_files(&mut self, ui: &mut egui::Ui) {
        theme::section_heading(
            ui,
            "Files",
            Some("Select files and folders to copy."),
        );
        ui.add_space(12.0);

        if !self.source_ready() {
            ui.label(
                egui::RichText::new("Choose a source folder first (Source tab).")
                    .color(colors::TEXT_SECONDARY),
            );
            return;
        }

        if let Some(loc) = self.source_location.and_then(|id| self.location(id)) {
            let host = self
                .computer(loc.computer_id)
                .map(|c| c.name.as_str())
                .unwrap_or("?");
            let folder = folder_display_name(&loc.path, &loc.name);
            widgets::wrapped_label(
                ui,
                egui::RichText::new(format!("{host} — {folder}"))
                    .size(12.0)
                    .color(colors::TEXT_SECONDARY),
            );
            ui.add_space(8.0);
        }

        theme::card_frame().show(ui, |ui| {
            widgets::constrain_content(ui);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!("{} selected", self.selected.len()))
                        .color(colors::TEXT_SECONDARY),
                );
                if widgets::secondary_button(ui, "Select all").clicked() {
                    for e in &self.entries {
                        self.selected.insert(e.name.clone());
                    }
                }
                if widgets::secondary_button(ui, "Clear").clicked() {
                    self.selected.clear();
                }
                if widgets::secondary_button(ui, "Refresh").clicked() {
                    self.refresh_file_list();
                }
                if self.listing {
                    ui.spinner();
                }
            });

            if let Some(err) = &self.list_error {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    Icon::Xmark.ui(ui, colors::ERROR);
                    ui.add(
                        egui::Label::new(egui::RichText::new(err).color(colors::ERROR)).wrap(),
                    );
                });
            }

            ui.add_space(8.0);
            let list_height = (ui.available_height() - 8.0).max(120.0);
            egui::ScrollArea::vertical()
                .id_salt("file_list")
                .auto_shrink([false, false])
                .max_height(list_height)
                .show(ui, |ui| {
                    widgets::constrain_content(ui);
                    if self.entries.is_empty() && !self.listing {
                        ui.vertical_centered(|ui| {
                            ui.add_space(24.0);
                            Icon::Document.ui(ui, colors::TEXT_SECONDARY);
                            ui.label(
                                egui::RichText::new("No files in this folder")
                                    .color(colors::TEXT_SECONDARY),
                            );
                        });
                    } else {
                        for e in &self.entries {
                            let mut on = self.selected.contains(&e.name);
                            ui.horizontal_wrapped(|ui| {
                                widgets::constrain_content(ui);
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
                                let checkbox = ui.add(egui::Checkbox::new(&mut on, ""));
                                let label_resp = ui.add(egui::Label::new(label).wrap());
                                if label_resp.clicked() {
                                    on = !on;
                                }
                                if checkbox.changed() || label_resp.clicked() {
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
    }

    fn ui_step_destination(&mut self, ui: &mut egui::Ui) {
        if theme::section_heading_with_action(
            ui,
            "Destination",
            Some("Choose a saved location, or add a new one."),
            "Add location",
        ) {
            self.open_location_picker(Side::Dest);
        }
        ui.add_space(12.0);
        self.ui_location_tiles(ui, Side::Dest);
        ui.add_space(16.0);
        if widgets::secondary_button(ui, "Check access").clicked() {
            self.run_preflight();
        }
        if let Some(preflight) = &self.preflight_ok {
            ui.add_space(8.0);
            widgets::status_message(ui, preflight);
        }
    }

    fn ui_location_tiles(&mut self, ui: &mut egui::Ui, side: Side) {
        widgets::constrain_content(ui);
        let selected_id = match side {
            Side::Source => self.source_location,
            Side::Dest => self.dest_location,
        };

        let mut pick: Option<Uuid> = None;
        let mut delete_id: Option<Uuid> = None;
        let mut reorder: Option<(Uuid, Uuid, Option<Uuid>)> = None;

        let mut by_host: HashMap<Uuid, Vec<&Location>> = HashMap::new();
        for loc in &self.locations {
            by_host.entry(loc.computer_id).or_default().push(loc);
        }
        for locs in by_host.values_mut() {
            locs.sort_by_key(|l| l.sort_order);
        }

        let tile_w = 136.0;

        for computer in &self.computers {
            let Some(locs) = by_host.get(&computer.id) else {
                continue;
            };
            if locs.is_empty() {
                continue;
            }

            let host_color = host_color(computer.id);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                Icon::Computer.ui(ui, host_color);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&computer.name)
                            .strong()
                            .color(colors::TEXT_PRIMARY),
                    )
                    .wrap(),
                );
            });
            ui.add_space(6.0);

            ui.with_layout(
                egui::Layout::left_to_right(egui::Align::TOP).with_main_wrap(true),
                |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                    for loc in locs {
                        let folder_name = folder_display_name(&loc.path, &loc.name);
                        let selected = selected_id == Some(loc.id);
                        let payload = LocationDragPayload {
                            location_id: loc.id,
                            computer_id: computer.id,
                        };

                        let tile = location_tile(
                            ui,
                            egui::Id::new(("loc_tile", side, loc.id)),
                            &folder_name,
                            host_color,
                            selected,
                            tile_w,
                            payload,
                        );

                        if tile.delete {
                            delete_id = Some(loc.id);
                        } else if tile.selected {
                            pick = Some(loc.id);
                        }
                        if let Some(drag) = tile.dropped {
                            reorder = Some((computer.id, drag.location_id, Some(loc.id)));
                        }
                    }

                    if let Some(drag) = location_tile_drop_tail(ui, computer.id) {
                        if drag.computer_id == computer.id {
                            reorder = Some((computer.id, drag.location_id, None));
                        }
                    }
                },
            );
            ui.add_space(14.0);
        }

        if let Some((computer_id, dragged_id, before_id)) = reorder {
            self.reorder_location_tiles(computer_id, dragged_id, before_id);
        }
        if let Some(id) = delete_id {
            self.delete_location_tile(id, side);
        } else if let Some(id) = pick {
            self.apply_location_selection(Some(id), side);
            if side == Side::Source {
                self.ensure_files_listed();
            }
        }
    }

    fn delete_location_tile(&mut self, id: Uuid, side: Side) {
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
        let _ = side;
    }

    fn reorder_location_tiles(
        &mut self,
        computer_id: Uuid,
        dragged_id: Uuid,
        before_id: Option<Uuid>,
    ) {
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

    fn open_location_picker(&mut self, side: Side) {
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
        });
        self.computer_msg.clear();
    }

    fn ui_location_picker(&mut self, ctx: &egui::Context) {
        let Some(picker) = &self.location_picker else {
            return;
        };
        let side = picker.side;
        let title = match side {
            Side::Source => "New source location",
            Side::Dest => "New destination location",
        };

        let mut open = true;
        let mut close = false;
        let mut use_path = false;
        let mut browse = false;
        let mut set_computer: Option<Uuid> = None;
        let mut save_discovered: Option<(String, String, u16)> = None;
        let mut add_computer = false;

        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([520.0, 560.0])
            .frame(theme::card_frame())
            .show(ctx, |ui| {
                widgets::constrain_content(ui);

                theme::section_heading(
                    ui,
                    "Choose a host",
                    Some("Select where the folder lives."),
                );
                ui.add_space(8.0);

                ui.horizontal_wrapped(|ui| {
                    widgets::constrain_content(ui);
                    for c in &self.computers {
                        let selected = self
                            .location_picker
                            .as_ref()
                            .and_then(|p| p.computer_id)
                            == Some(c.id);
                        let label = if c.is_local {
                            format!("{} (This Mac)", c.name)
                        } else {
                            c.name.clone()
                        };
                        let btn = ui.add(
                            egui::Button::new(
                                egui::RichText::new(label)
                                    .color(if selected {
                                        egui::Color32::WHITE
                                    } else {
                                        colors::TEXT_PRIMARY
                                    })
                                    .size(12.5),
                            )
                            .fill(if selected {
                                colors::ACCENT
                            } else {
                                egui::Color32::from_rgb(242, 242, 247)
                            })
                            .stroke(egui::Stroke::new(1.0_f32, colors::SEPARATOR))
                            .corner_radius(egui::CornerRadius::same(8)),
                        );
                        if btn.clicked() {
                            set_computer = Some(c.id);
                        }
                    }
                });

                let discovered = self
                    .discovery
                    .as_ref()
                    .map(|d| d.hosts())
                    .unwrap_or_default();
                if !discovered.is_empty() {
                    ui.add_space(12.0);
                    widgets::field_label(ui, "Discovered on network");
                    for h in &discovered {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            Icon::Network.ui(ui, colors::ACCENT);
                            ui.label(format!("{} — {}:{}", h.name, h.host, h.port));
                            if widgets::secondary_button(ui, "Add host").clicked() {
                                save_discovered =
                                    Some((h.name.clone(), h.host.clone(), h.port));
                            }
                        });
                    }
                }

                ui.add_space(12.0);
                egui::CollapsingHeader::new("Add host manually").show(ui, |ui| {
                    widgets::field_label(ui, "Display name");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_name)
                            .desired_width(widgets::combo_width(ui)),
                    );
                    widgets::field_label(ui, "SSH destination");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_ssh)
                            .desired_width(widgets::combo_width(ui)),
                    );
                    widgets::field_label(ui, "Port (optional)");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_port).desired_width(120.0),
                    );
                    ui.add_space(8.0);
                    if widgets::secondary_button(ui, "Add host").clicked() {
                        add_computer = true;
                    }
                });

                if !self.computer_msg.is_empty() {
                    ui.add_space(6.0);
                    ui.label(&self.computer_msg);
                }

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(12.0);

                theme::section_heading(
                    ui,
                    "Choose a folder",
                    Some("Browse or enter an absolute path."),
                );
                ui.add_space(8.0);
                widgets::field_label(ui, "Path");
                if let Some(picker) = &mut self.location_picker {
                    let _ = widgets::path_field(ui, &mut picker.path_edit, "absolute path…");
                }

                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| {
                    if widgets::icon_button(ui, Icon::FolderOpen, "Browse…").clicked() {
                        browse = true;
                    }
                    if widgets::primary_button(ui, "Use path").clicked() {
                        use_path = true;
                    }
                    if widgets::secondary_button(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            });

        if !open {
            close = true;
        }
        if let Some(cid) = set_computer {
            if let Some(picker) = &mut self.location_picker {
                picker.computer_id = Some(cid);
            }
        }
        if let Some((name, host, port)) = save_discovered {
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
        if add_computer {
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
        if browse {
            let computer_id = self
                .location_picker
                .as_ref()
                .and_then(|p| p.computer_id);
            if let Some(cid) = computer_id {
                let target = match side {
                    Side::Source => BrowseTarget::Source,
                    Side::Dest => BrowseTarget::Dest,
                };
                self.open_folder_browse(cid, target);
            } else {
                self.computer_msg = "Choose a host first".into();
            }
        } else if use_path {
            let (computer_id, path) = self
                .location_picker
                .as_ref()
                .map(|p| (p.computer_id, p.path_edit.trim().to_string()))
                .unwrap_or((None, String::new()));
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
        } else if close {
            self.location_picker = None;
        }
    }

    fn apply_location_selection(&mut self, location_id: Option<Uuid>, side: Side) {
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

    fn ui_history(&mut self, ui: &mut egui::Ui) {
        widgets::constrain_content(ui);
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
            widgets::constrain_content(ui);
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
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                    widgets::constrain_content(ui);
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

                        ui.horizontal_wrapped(|ui| {
                            widgets::constrain_content(ui);
                            ui.vertical(|ui| {
                                ui.set_max_width(ui.available_width());
                                ui.horizontal_wrapped(|ui| {
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
                        if let Some(err) = &j.error_summary {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                Icon::Xmark.ui(ui, colors::ERROR);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(err).color(colors::ERROR),
                                    )
                                    .wrap(),
                                );
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
