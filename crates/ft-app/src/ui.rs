use crate::icons::{Glyph, Icon};
use crate::state::{AccessCheck, AppState, BrowseTarget, NavTab, Side};
use crate::util::{
    folder_display_name, format_bytes, host_color, progress_detail, progress_fraction, status_kind,
};
use dioxus::prelude::*;
use std::path::PathBuf;
use uuid::Uuid;

pub fn app() -> Element {
    let mut state = use_signal(|| AppState::new().expect("open File Transfer store"));
    use_context_provider(|| state);
    #[cfg(target_os = "macos")]
    crate::macos::attach_menubar();

    use_future(move || async move {
        loop {
            let busy = {
                let s = state.read();
                s.transferring || matches!(s.access, AccessCheck::Testing)
            };
            let ms = if busy { 100 } else { 250 };
            futures_timer::Delay::new(std::time::Duration::from_millis(ms)).await;
            state.write().poll_bg();
        }
    });

    let tab = state.read().tab;
    let show_picker = state.read().location_picker.is_some();
    let show_browser = state.read().folder_browser.is_some();

    rsx! {
        div { class: "app",
            TitlebarDrag {}
            Sidebar {}
            section { class: "main",
                div { class: "page",
                    PageHeader {}
                    div { class: "page-scroll",
                        match tab {
                            NavTab::Source => rsx! { SourceStep {} },
                            NavTab::Files => rsx! { FilesStep {} },
                            NavTab::Destination => rsx! { DestinationStep {} },
                        }
                    }
                    WizardBar {}
                }
            }
            Footer {}
        }
        if show_picker {
            LocationPickerSheet {}
        }
        if show_browser {
            FolderBrowserSheet {}
        }
    }
}

#[component]
fn TitlebarDrag() -> Element {
    let desktop = dioxus::desktop::use_window();
    rsx! {
        div {
            class: "titlebar-drag",
            onmousedown: move |_| desktop.drag(),
        }
    }
}

#[component]
fn Sidebar() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let tab = state.read().tab;
    let transferring = state.read().transferring;
    let source_host = state.read().side_host_name(Side::Source);
    let source_folder = state.read().side_folder_name(Side::Source);
    let source_folder_title = state.read().side_folder_title(Side::Source);
    let dest_host = state.read().side_host_name(Side::Dest);
    let dest_folder = state.read().side_folder_name(Side::Dest);
    let dest_folder_title = state.read().side_folder_title(Side::Dest);
    let copy_items = state.read().copy_items();
    let copy_n = copy_items.len();
    let copy_label = if copy_n == 0 {
        "—".to_string()
    } else if copy_n == 1 {
        "1 item".to_string()
    } else {
        format!("{copy_n} items")
    };
    let copy_stats = state.read().copy_stats_label();
    let access = state.read().access.clone();
    let status_kind = access.kind();
    let status_label = access.label();
    let status_detail = access.detail().to_string();
    rsx! {
        aside { class: "sidebar",
            div { class: "sidebar-traffic" }
            div { class: "brand",
                div { class: "brand-mark",
                    Icon { kind: Glyph::Transfer }
                }
                h1 { "File Transfer" }
            }
            nav { class: "nav",
                NavButton {
                    tab: NavTab::Source,
                    current: tab,
                    kind: Glyph::Computer,
                    label: "Source",
                    onclick: move |_| state.write().set_tab(NavTab::Source),
                }
                NavButton {
                    tab: NavTab::Files,
                    current: tab,
                    kind: Glyph::Document,
                    label: "Files",
                    onclick: move |_| state.write().set_tab(NavTab::Files),
                }
                NavButton {
                    tab: NavTab::Destination,
                    current: tab,
                    kind: Glyph::Folder,
                    label: "Destination",
                    onclick: move |_| state.write().set_tab(NavTab::Destination),
                }
            }
            div { class: "sidebar-panel",
                div { class: "sidebar-panel-title", "Access" }
                div { class: "access-row",
                    span { class: "access-k", "Source Host" }
                    span { class: "access-v", title: "{source_host}", "{source_host}" }
                }
                div { class: "access-row",
                    span { class: "access-k", "Source Folder" }
                    span { class: "access-v", title: "{source_folder_title}", "{source_folder}" }
                }
                div { class: "access-copy",
                    div { class: "access-row access-copy-trigger",
                        span { class: "access-k", "Files" }
                        span { class: "access-v access-copy-value",
                            Icon { kind: Glyph::Document }
                            span { "{copy_label}" }
                        }
                    }
                    div { class: "access-copy-popup",
                        div { class: "access-copy-popup-title", "Files to copy" }
                        if copy_n == 0 {
                            div { class: "access-copy-popup-empty", "No files selected" }
                        } else {
                            div { class: "access-copy-popup-list",
                                for item in copy_items {
                                    {
                                        let label = if item.is_dir {
                                            format!("{}/", item.name)
                                        } else {
                                            item.name.clone()
                                        };
                                        let glyph = if item.is_dir {
                                            Glyph::Folder
                                        } else {
                                            Glyph::Document
                                        };
                                        rsx! {
                                            div { class: "access-copy-popup-row",
                                                Icon { kind: glyph }
                                                span { title: "{label}", "{label}" }
                                            }
                                        }
                                    }
                                }
                            }
                            div { class: "access-copy-popup-foot", "{copy_stats}" }
                        }
                    }
                }
                div { class: "access-row",
                    span { class: "access-k", "Destination Host" }
                    span { class: "access-v", title: "{dest_host}", "{dest_host}" }
                }
                div { class: "access-row",
                    span { class: "access-k", "Destination Folder" }
                    span { class: "access-v", title: "{dest_folder_title}", "{dest_folder}" }
                }
                div { class: "access-status-block", title: "{status_detail}",
                    div { class: "access-status-caption", "Access status" }
                    div { class: "access-status-line",
                        span { class: "access-status {status_kind}",
                            if matches!(access, AccessCheck::Untested) {
                                Icon { kind: Glyph::Help }
                            } else if matches!(access, AccessCheck::Testing) {
                                span { class: "spinner" }
                            } else if matches!(access, AccessCheck::Accessible { .. }) {
                                Icon { kind: Glyph::Check }
                            } else {
                                Icon { kind: Glyph::Close }
                            }
                        }
                        span { class: "access-status-text {status_kind}", "{status_label}" }
                    }
                }
            }
            div { class: "sidebar-spacer" }
            button {
                class: "btn sidebar-reset",
                disabled: transferring,
                onclick: move |_| state.write().reset_transfer(),
                "Reset"
            }
            div { class: "sidebar-foot", "Direct rsync over SSH" }
        }
    }
}

#[component]
fn NavButton(
    tab: NavTab,
    current: NavTab,
    kind: Glyph,
    label: &'static str,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    let class = if tab == current {
        "nav-item is-selected"
    } else {
        "nav-item"
    };
    rsx! {
        button { class, onclick: move |evt| onclick.call(evt),
            Icon { kind }
            span { "{label}" }
        }
    }
}

#[component]
fn PageHeader() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let tab = state.read().tab;
    let action = matches!(tab, NavTab::Source | NavTab::Destination);
    let locked = state.read().selections_locked();
    rsx! {
        header { class: "page-header",
            div {
                h2 { "{tab.title()}" }
                p { "{tab.subtitle()}" }
            }
            if action {
                button {
                    class: "btn btn-primary",
                    disabled: locked,
                    onclick: move |_| {
                        let side = if tab == NavTab::Source { Side::Source } else { Side::Dest };
                        state.write().open_location_picker(side);
                    },
                    "Add Location"
                }
            }
        }
    }
}

#[component]
fn WizardBar() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let tab = state.read().tab;
    let can_advance = state.read().can_advance();
    rsx! {
        div { class: "wizard-bar",
            if tab.prev().is_some() {
                button { class: "btn", onclick: move |_| state.write().go_back(), "Back" }
            }
            if tab.next().is_some() {
                button {
                    class: "btn btn-primary",
                    disabled: !can_advance,
                    onclick: move |_| state.write().go_next(),
                    "Continue"
                }
            }
        }
    }
}

#[component]
fn SourceStep() -> Element {
    rsx! { LocationTiles { side: Side::Source } }
}

#[component]
fn DestinationStep() -> Element {
    rsx! { LocationTiles { side: Side::Dest } }
}

#[component]
fn FilesStep() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let source_ready = state.read().source_ready();
    let label = state.read().source_folder_label();
    let listing = state.read().listing;
    let list_error = state.read().list_error.clone();
    let entries = state.read().entries.clone();
    let selected = state.read().selected.clone();
    let selected_n = selected.len();
    let locked = state.read().selections_locked();

    if !source_ready {
        return rsx! {
            div { class: "empty",
                Icon { kind: Glyph::Folder }
                span { "Choose a source folder first." }
            }
        };
    }

    rsx! {
        if let Some(label) = label {
            p { class: "caption", "{label}" }
        }
        div { class: if locked { "file-card is-locked" } else { "file-card" },
            div { class: "file-toolbar",
                span { class: "file-count", "{selected_n} selected" }
                button { class: "btn", disabled: locked, onclick: move |_| state.write().select_all_files(), "Select All" }
                button { class: "btn", disabled: locked, onclick: move |_| state.write().clear_file_selection(), "Clear" }
                button { class: "btn", disabled: locked, onclick: move |_| state.write().refresh_file_list(),
                    Icon { kind: Glyph::Refresh }
                    "Refresh"
                }
                if listing {
                    div { class: "spinner" }
                }
            }
            if let Some(err) = list_error {
                div { class: "msg err", style: "padding: 8px 12px;",
                    Icon { kind: Glyph::Close }
                    span { "{err}" }
                }
            }
            div { class: "file-list",
                if entries.is_empty() && !listing {
                    div { class: "empty",
                        Icon { kind: Glyph::Document }
                        span { "No files in this folder" }
                    }
                } else {
                    for entry in entries {
                        {
                            let name = entry.name.clone();
                            let on = selected.contains(&name);
                            let class = if on {
                                "file-row is-on"
                            } else if locked {
                                "file-row is-locked"
                            } else {
                                "file-row"
                            };
                            let glyph = if entry.is_dir { Glyph::Folder } else { Glyph::Document };
                            let size = if entry.is_dir {
                                String::new()
                            } else {
                                format_bytes(entry.size)
                            };
                            let label = if entry.is_dir {
                                format!("{}/", entry.name)
                            } else {
                                entry.name.clone()
                            };
                            rsx! {
                                    label {
                                    class,
                                    key: "{name}",
                                    input {
                                        r#type: "checkbox",
                                        checked: on,
                                        disabled: locked,
                                        onchange: move |_| state.write().toggle_file(&name),
                                    }
                                    Icon { kind: glyph }
                                    span { class: "file-name", "{label}" }
                                    span { class: "file-size", "{size}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn LocationTiles(side: Side) -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let mut dragging = use_signal(|| None::<(Uuid, Uuid)>);
    let mut drop_over = use_signal(|| None::<Uuid>);
    let mut tail_over = use_signal(|| None::<Uuid>);
    let groups = state.read().location_groups();
    let selected_id = match side {
        Side::Source => state.read().source_location,
        Side::Dest => state.read().dest_location,
    };
    let locked = state.read().selections_locked();

    if groups.is_empty() {
        return rsx! {
            div { class: "empty",
                Icon { kind: Glyph::Folder }
                span { "No saved locations yet. Add one to get started." }
            }
        };
    }

    rsx! {
        for (computer, locs) in groups {
            {
                let computer_id = computer.id;
                let color = host_color(computer.id);
                rsx! {
                    div {
                        class: "host-block",
                        key: "{computer.id}",
                        style: "--host: {color}",
                        div { class: "host-label",
                            Icon { kind: Glyph::Computer }
                            span { "{computer.name}" }
                            div { class: "host-rule" }
                        }
                        div { class: "tiles",
                            for loc in locs {
                                {
                                    let loc_id = loc.id;
                                    let name = folder_display_name(&loc.path, &loc.name);
                                    let selected = selected_id == Some(loc_id);
                                    let class = {
                                        let mut c = String::from("tile");
                                        if selected {
                                            c.push_str(" is-selected");
                                        }
                                        if locked {
                                            c.push_str(" is-locked");
                                        }
                                        if dragging().is_some_and(|(id, _)| id == loc_id) {
                                            c.push_str(" is-dragging");
                                        }
                                        if drop_over() == Some(loc_id) {
                                            c.push_str(" is-drop");
                                        }
                                        c
                                    };
                                    rsx! {
                                        div {
                                            class,
                                            key: "{loc_id}",
                                            draggable: if locked { "false" } else { "true" },
                                            onclick: move |_| state.write().select_location(loc_id, side),
                                            ondragstart: move |_| dragging.set(Some((loc_id, computer_id))),
                                            ondragend: move |_| {
                                                dragging.set(None);
                                                drop_over.set(None);
                                                tail_over.set(None);
                                            },
                                            ondragover: move |evt| {
                                                evt.prevent_default();
                                                drop_over.set(Some(loc_id));
                                            },
                                            ondragleave: move |_| {
                                                if drop_over() == Some(loc_id) {
                                                    drop_over.set(None);
                                                }
                                            },
                                            ondrop: move |evt| {
                                                evt.prevent_default();
                                                if let Some((drag_id, drag_cid)) = dragging() {
                                                    if drag_cid == computer_id {
                                                        state.write().reorder_location_tiles(
                                                            computer_id,
                                                            drag_id,
                                                            Some(loc_id),
                                                        );
                                                    }
                                                }
                                                dragging.set(None);
                                                drop_over.set(None);
                                            },
                                            button {
                                                class: "tile-delete",
                                                r#type: "button",
                                                disabled: locked,
                                                onclick: move |evt| {
                                                    evt.stop_propagation();
                                                    state.write().delete_location_tile(loc_id);
                                                },
                                                Icon { kind: Glyph::Close }
                                            }
                                            div { class: "tile-icon",
                                                Icon { kind: Glyph::Folder }
                                            }
                                            div { class: "tile-name", title: "{loc.path.display()}", "{name}" }
                                        }
                                    }
                                }
                            }
                            button {
                                class: "tile tile-add",
                                r#type: "button",
                                disabled: locked,
                                onclick: move |_| state.write().browse_on_host(computer_id, side),
                                div { class: "tile-icon",
                                    Icon { kind: Glyph::Plus }
                                }
                                div { class: "tile-name", "Add folder" }
                            }
                            {
                                let tail_class = if tail_over() == Some(computer_id) {
                                    "drop-tail is-drop"
                                } else {
                                    "drop-tail"
                                };
                                rsx! {
                                    div {
                                        class: tail_class,
                                        ondragover: move |evt| {
                                            evt.prevent_default();
                                            tail_over.set(Some(computer_id));
                                        },
                                        ondragleave: move |_| {
                                            if tail_over() == Some(computer_id) {
                                                tail_over.set(None);
                                            }
                                        },
                                        ondrop: move |evt| {
                                            evt.prevent_default();
                                            if let Some((drag_id, drag_cid)) = dragging() {
                                                if drag_cid == computer_id {
                                                    state.write().reorder_location_tiles(
                                                        computer_id,
                                                        drag_id,
                                                        None,
                                                    );
                                                }
                                            }
                                            dragging.set(None);
                                            tail_over.set(None);
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Footer() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let transferring = state.read().transferring;
    let can_transfer = state.read().transfer_ready();
    let status_line = state.read().status_line.clone();
    let progress = state.read().progress.clone();
    let status = if status_line.is_empty() {
        if transferring {
            "Transferring…".to_string()
        } else {
            "Ready".to_string()
        }
    } else {
        status_line.clone()
    };
    let kind = status_kind(&status_line, transferring);
    let detail = progress_detail(&progress, transferring);
    let frac = progress_fraction(&progress, transferring);
    let fill_class = if frac < 0.0 {
        "progress-fill is-indeterminate"
    } else {
        "progress-fill"
    };
    let fill_style = if frac < 0.0 {
        String::new()
    } else {
        format!("width: {:.0}%", (frac * 100.0).clamp(0.0, 100.0))
    };

    rsx! {
        footer { class: "footer",
            div { class: "footer-row",
                div { class: "footer-status {kind}",
                    if kind == "ok" {
                        Icon { kind: Glyph::Check }
                    }
                    if kind == "err" {
                        Icon { kind: Glyph::Close }
                    }
                    span { "{status}" }
                }
                div { class: "footer-actions",
                    if transferring {
                        button { class: "btn", onclick: move |_| state.read().request_cancel(), "Cancel" }
                    }
                    button {
                        class: "btn btn-primary",
                        disabled: !can_transfer,
                        onclick: move |_| state.write().start_transfer(),
                        "Transfer"
                    }
                }
            }
            div { class: "progress-meta", "{detail}" }
            div { class: "progress-track",
                div { class: fill_class, style: "{fill_style}" }
            }
        }
    }
}

#[component]
fn LocationPickerSheet() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let computers = state.read().computers.clone();
    let picker_computer = state
        .read()
        .location_picker
        .as_ref()
        .and_then(|p| p.computer_id);
    let side = state
        .read()
        .location_picker
        .as_ref()
        .map(|p| p.side)
        .unwrap_or(Side::Source);
    let title = match side {
        Side::Source => "New Source Location",
        Side::Dest => "New Destination Location",
    };
    let path_edit = state
        .read()
        .location_picker
        .as_ref()
        .map(|p| p.path_edit.clone())
        .unwrap_or_default();
    let show_manual = state
        .read()
        .location_picker
        .as_ref()
        .map(|p| p.show_manual_host)
        .unwrap_or(false);
    let discovered = state.read().discovered_hosts();
    let new_name = state.read().new_name.clone();
    let new_ssh = state.read().new_ssh.clone();
    let new_port = state.read().new_port.clone();
    let computer_msg = state.read().computer_msg.clone();

    rsx! {
        div {
            class: "backdrop",
            onclick: move |_| state.write().location_picker = None,
            div {
                class: "sheet",
                onclick: move |evt| evt.stop_propagation(),
                h3 { "{title}" }
                p { class: "sheet-sub", "Select where the folder lives, then choose a path." }

                div { class: "field-label", "Host" }
                div { class: "chips",
                    for c in computers {
                        {
                            let id = c.id;
                            let selected = picker_computer == Some(id);
                            let class = if selected { "chip is-selected" } else { "chip" };
                            let label = if c.is_local {
                                format!("{} (This Mac)", c.name)
                            } else {
                                c.name.clone()
                            };
                            rsx! {
                                button {
                                    class,
                                    key: "{id}",
                                    onclick: move |_| state.write().picker_set_computer(id),
                                    "{label}"
                                }
                            }
                        }
                    }
                }

                if !discovered.is_empty() {
                    div { class: "field-label", style: "margin-top: 14px;", "Discovered on Network" }
                    div { class: "discovered",
                        for host in discovered {
                            {
                                let name = host.name.clone();
                                let dest = host.host.clone();
                                let port = host.port;
                                rsx! {
                                    div { class: "discovered-row", key: "{name}-{dest}-{port}",
                                        Icon { kind: Glyph::Network }
                                        span { "{name} — {dest}:{port}" }
                                        button {
                                            class: "btn",
                                            onclick: move |_| state.write().add_discovered_host(
                                                name.clone(),
                                                dest.clone(),
                                                port,
                                            ),
                                            "Add Host"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                button {
                    class: "disclosure",
                    onclick: move |_| {
                        if let Some(p) = state.write().location_picker.as_mut() {
                            p.show_manual_host = !p.show_manual_host;
                        }
                    },
                    if show_manual { "Hide manual host" } else { "Add host manually" }
                }
                if show_manual {
                    div { class: "manual",
                        div { class: "field",
                            label { "Display name" }
                            input {
                                r#type: "text",
                                value: "{new_name}",
                                oninput: move |e| state.write().new_name = e.value(),
                            }
                        }
                        div { class: "field",
                            label { "SSH destination" }
                            input {
                                r#type: "text",
                                value: "{new_ssh}",
                                oninput: move |e| state.write().new_ssh = e.value(),
                            }
                        }
                        div { class: "field",
                            label { "Port (optional)" }
                            input {
                                r#type: "text",
                                value: "{new_port}",
                                oninput: move |e| state.write().new_port = e.value(),
                            }
                        }
                        button { class: "btn", onclick: move |_| state.write().add_manual_host(), "Add Host" }
                    }
                }

                if !computer_msg.is_empty() {
                    div { class: "msg", "{computer_msg}" }
                }

                div { class: "section-gap" }
                div { class: "field-label", "Folder" }
                div { class: "path-row",
                    input {
                        r#type: "text",
                        placeholder: "absolute path…",
                        value: "{path_edit}",
                        oninput: move |e| {
                            if let Some(p) = state.write().location_picker.as_mut() {
                                p.path_edit = e.value();
                            }
                        },
                    }
                }
                div { class: "sheet-actions",
                    button { class: "btn", onclick: move |_| state.write().location_picker = None, "Cancel" }
                    button {
                        class: "btn",
                        onclick: move |_| state.write().picker_browse(),
                        Icon { kind: Glyph::FolderOpen }
                        "Browse…"
                    }
                    button { class: "btn btn-primary", onclick: move |_| state.write().picker_use_path(), "Use Path" }
                }
            }
        }
    }
}

#[component]
fn FolderBrowserSheet() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let title = state
        .read()
        .folder_browser
        .as_ref()
        .map(|b| match b.target {
            BrowseTarget::Source => "Browse Source Folder",
            BrowseTarget::Dest => "Browse Destination Folder",
        })
        .unwrap_or("Browse Folder");
    let computer_id = state.read().folder_browser.as_ref().map(|b| b.computer_id);
    let computer_name = computer_id
        .and_then(|id| state.read().computer(id).map(|c| c.name.clone()))
        .unwrap_or_else(|| "remote".into());
    let path_edit = state
        .read()
        .folder_browser
        .as_ref()
        .map(|b| b.path_edit.clone())
        .unwrap_or_default();
    let current_path = state
        .read()
        .folder_browser
        .as_ref()
        .map(|b| b.current_path.display().to_string())
        .unwrap_or_default();
    let loading = state
        .read()
        .folder_browser
        .as_ref()
        .map(|b| b.loading)
        .unwrap_or(false);
    let error = state
        .read()
        .folder_browser
        .as_ref()
        .and_then(|b| b.error.clone());
    let entries = state
        .read()
        .folder_browser
        .as_ref()
        .map(|b| b.entries.clone())
        .unwrap_or_default();
    let empty = entries.is_empty() && !loading;

    rsx! {
        div {
            class: "backdrop",
            onclick: move |_| state.write().folder_browser = None,
            div {
                class: "sheet",
                onclick: move |evt| evt.stop_propagation(),
                h3 { "{title}" }
                p { class: "sheet-sub", "{computer_name}" }
                div { class: "path-row",
                    input {
                        r#type: "text",
                        value: "{path_edit}",
                        oninput: move |e| {
                            if let Some(b) = state.write().folder_browser.as_mut() {
                                b.path_edit = e.value();
                            }
                        },
                    }
                    button {
                        class: "btn",
                        onclick: move |_| {
                            let p = state
                                .read()
                                .folder_browser
                                .as_ref()
                                .map(|b| b.path_edit.trim().to_string())
                                .unwrap_or_default();
                            if !p.is_empty() {
                                state.write().browser_go_path(PathBuf::from(p));
                            }
                        },
                        "Go"
                    }
                }
                div { class: "toolbar", style: "margin-top: 10px;",
                    button { class: "btn", onclick: move |_| state.write().browser_go_home(),
                        Icon { kind: Glyph::Home }
                        "Home"
                    }
                    button { class: "btn", onclick: move |_| state.write().browser_go_up(), "Up" }
                    button { class: "btn", onclick: move |_| state.write().refresh_folder_browser(),
                        Icon { kind: Glyph::Refresh }
                        "Refresh"
                    }
                    if loading {
                        div { class: "spinner" }
                    }
                }
                if let Some(err) = error {
                    div { class: "msg err",
                        Icon { kind: Glyph::Close }
                        span { "{err}" }
                    }
                }
                p { class: "caption", "{current_path}" }
                div { class: "browse-dirs",
                    if empty {
                        div { class: "empty", span { "No subfolders" } }
                    }
                    for entry in entries {
                        {
                            let name = entry.name.clone();
                            let open_name = name.clone();
                            rsx! {
                                button {
                                    class: "browse-row",
                                    key: "{name}",
                                    onclick: move |_| state.write().browser_enter(name.clone()),
                                    Icon { kind: Glyph::Folder }
                                    span { "{open_name}/" }
                                }
                            }
                        }
                    }
                }
                div { class: "sheet-actions",
                    button { class: "btn", onclick: move |_| state.write().folder_browser = None, "Cancel" }
                    button { class: "btn btn-primary", onclick: move |_| state.write().browser_select(), "Select Folder" }
                }
            }
        }
    }
}
