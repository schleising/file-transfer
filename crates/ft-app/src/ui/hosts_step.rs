use crate::icons::{Glyph, Icon};
use crate::state::{AppState, HostFormTarget};
use crate::util::{host_color, parse_port_field, HOST_PALETTE};
use dioxus::prelude::*;
use uuid::Uuid;

struct HostRow {
    id: Uuid,
    color: &'static str,
    name: String,
    sub: String,
    meta: String,
    selected: bool,
}

#[component]
pub fn HostsStep() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let computers = state.read().computers.clone();
    let locations = state.read().locations.clone();
    let form = state.read().host_form.clone();
    let locked = state.read().selections_locked();
    let discovered = state.read().unmatched_discoveries();
    let pending = state.read().discovery_pending();
    let add_enabled = state.read().host_add_enabled();
    let save_enabled = state.read().host_save_enabled();

    let Some(form) = form else {
        return rsx! { div { class: "hosts-step" } };
    };

    let selected_id = match form.target {
        HostFormTarget::Existing(id) => Some(id),
        HostFormTarget::New => None,
    };
    let selected = selected_id.and_then(|id| computers.iter().find(|computer| computer.id == id));
    let is_local = selected.is_some_and(|computer| computer.is_local);
    let is_new = selected_id.is_none();
    let on_network = selected_id.is_some_and(|id| state.read().host_on_network(id));
    let folder_n = selected_id
        .map(|id| locations.iter().filter(|loc| loc.computer_id == id).count())
        .unwrap_or(0);
    let prompt = selected
        .map(|computer| delete_prompt(&computer.name, folder_n))
        .unwrap_or_default();
    let message = if parse_port_field(&form.port).is_err() {
        "Port must be a number.".to_string()
    } else {
        form.message.clone()
    };
    let form_color = form.color.clone();
    let form_name = form.name.clone();
    let form_ssh = form.ssh.clone();
    let form_port = form.port.clone();
    let confirm = form.confirm_delete && !is_new && !is_local;
    let add_class = if is_new {
        "host-row add-row is-selected"
    } else {
        "host-row add-row"
    };
    let rows: Vec<HostRow> = computers
        .iter()
        .map(|computer| {
            let folders = locations
                .iter()
                .filter(|loc| loc.computer_id == computer.id)
                .count();
            HostRow {
                id: computer.id,
                color: host_color(computer.id, computer.color.as_deref()),
                name: computer.name.clone(),
                sub: host_line(computer),
                meta: folder_count_label(folders),
                selected: selected_id == Some(computer.id),
            }
        })
        .collect();

    rsx! {
        div { class: "hosts-step",
            if locked {
                p { class: "lock-note", "Hosts can be changed when the transfer finishes." }
            }
            div { class: "hosts-split",
                div { class: "hosts-card list-card",
                    div { class: "saved",
                        for row in rows {
                            {
                                let id = row.id;
                                let color = row.color;
                                let class = if row.selected {
                                    "host-row is-selected"
                                } else {
                                    "host-row"
                                };
                                rsx! {
                                    button {
                                        class: "{class}",
                                        key: "{id}",
                                        onclick: move |_| state.write().select_host(id),
                                        span { class: "dot", style: "background: {color}" }
                                        span {
                                            div { class: "host-name", "{row.name}" }
                                            div { class: "host-sub", "{row.sub}" }
                                        }
                                        span { class: "host-meta", "{row.meta}" }
                                    }
                                }
                            }
                        }
                        button {
                            class: "{add_class}",
                            disabled: locked,
                            onclick: move |_| state.write().select_new_host(),
                            Icon { kind: Glyph::Plus }
                            span { "Add host manually" }
                        }
                    }
                    div { class: "disc",
                        h4 { "Discovered on network" }
                        div { class: "discovered",
                            if discovered.is_empty() {
                                p { class: "editor-note",
                                    if pending {
                                        "Looking for SSH hosts…"
                                    } else {
                                        "No other SSH hosts on the network."
                                    }
                                }
                            }
                            for host in discovered {
                                {
                                    let name = host.name.clone();
                                    let dest = host.host.clone();
                                    let port = host.port;
                                    let label = format!("{name} — {dest}:{port}");
                                    rsx! {
                                        div { class: "discovered-row disc-row", key: "{label}",
                                            Icon { kind: Glyph::Network }
                                            span { "{label}" }
                                            button {
                                                class: "btn",
                                                disabled: locked,
                                                onclick: move |_| state.write().add_discovered_host(
                                                    name.clone(),
                                                    dest.clone(),
                                                    port,
                                                ),
                                                "Add"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "hosts-card editor",
                    if is_new {
                        p { class: "editor-note", "Name the computer and the SSH destination you already use in Terminal." }
                    }
                    div { class: "field",
                        label { "Display name" }
                        input {
                            r#type: "text",
                            value: "{form_name}",
                            placeholder: "Studio",
                            disabled: locked,
                            oninput: move |e| state.write().set_host_name(e.value()),
                        }
                    }
                    if !is_local {
                        div { class: "field",
                            label { "SSH destination" }
                            input {
                                r#type: "text",
                                value: "{form_ssh}",
                                placeholder: "user@host",
                                disabled: locked,
                                oninput: move |e| state.write().set_host_ssh(e.value()),
                            }
                        }
                        div { class: "field port-field",
                            label { "Port" }
                            input {
                                r#type: "text",
                                value: "{form_port}",
                                placeholder: "22",
                                disabled: locked,
                                oninput: move |e| state.write().set_host_port(e.value()),
                            }
                        }
                    }
                    div { class: "field-label", "Colour" }
                    div { class: "swatches",
                        for color in HOST_PALETTE {
                            {
                                let on = form_color.eq_ignore_ascii_case(color);
                                let class = if on { "swatch is-on" } else { "swatch" };
                                rsx! {
                                    button {
                                        class: "{class}",
                                        key: "{color}",
                                        style: "--c: {color}",
                                        disabled: locked,
                                        onclick: move |_| state.write().set_host_color(color),
                                    }
                                }
                            }
                        }
                    }
                    if on_network {
                        div { class: "net-line",
                            i {}
                            "On network"
                        }
                    }
                    if is_local {
                        p { class: "editor-note", "This Mac is always available." }
                    }
                    if !message.is_empty() {
                        div { class: "msg", "{message}" }
                    }
                    if confirm {
                        div { class: "confirm",
                            p { "{prompt}" }
                            button {
                                class: "btn",
                                disabled: locked,
                                onclick: move |_| state.write().cancel_delete_host(),
                                "Cancel"
                            }
                            button {
                                class: "btn btn-fill-danger",
                                disabled: locked,
                                onclick: move |_| state.write().confirm_delete_host(),
                                "Delete"
                            }
                        }
                    } else if is_new {
                        div { class: "editor-actions",
                            span { class: "grow" }
                            button {
                                class: "btn btn-primary",
                                disabled: !add_enabled,
                                onclick: move |_| state.write().add_manual_host(),
                                "Add Host"
                            }
                        }
                    } else {
                        div { class: "editor-actions",
                            if !is_local {
                                button {
                                    class: "btn btn-quiet-danger",
                                    disabled: locked,
                                    onclick: move |_| state.write().begin_delete_host(),
                                    "Delete Host"
                                }
                            }
                            span { class: "grow" }
                            button {
                                class: "btn btn-primary",
                                disabled: !save_enabled,
                                onclick: move |_| state.write().save_host(),
                                "Save"
                            }
                        }
                    }
                }
            }
        }
    }
}

fn host_line(computer: &ft_store::Computer) -> String {
    if computer.is_local {
        return "This Mac".into();
    }
    match computer.ssh_port {
        Some(port) if port != 22 => format!("{}:{port}", computer.ssh_destination),
        _ => computer.ssh_destination.clone(),
    }
}

fn folder_count_label(n: usize) -> String {
    match n {
        0 => "No folders".into(),
        1 => "1 folder".into(),
        n => format!("{n} folders"),
    }
}

fn delete_prompt(name: &str, folders: usize) -> String {
    match folders {
        0 => format!("Delete {name}?"),
        1 => format!("Delete {name}? Its 1 saved folder will be removed."),
        n => format!("Delete {name}? Its {n} saved folders will be removed."),
    }
}
