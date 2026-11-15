use std::path::Path;
use uuid::Uuid;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const CRATE_VERSIONS: [(&str, &str); 4] = [
    ("ft-app", APP_VERSION),
    ("ft-exec", ft_exec::VERSION),
    ("ft-store", ft_store::VERSION),
    ("ft-mdns", ft_mdns::VERSION),
];

pub fn format_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let x = n as f64;
    if x >= GB {
        format!("{:.2} GB", x / GB)
    } else if x >= MB {
        format!("{:.2} MB", x / MB)
    } else if x >= KB {
        format!("{:.1} KB", x / KB)
    } else {
        format!("{n} B")
    }
}

pub fn format_rate(bps: f64) -> String {
    if bps >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB/s", bps / (1024.0 * 1024.0 * 1024.0))
    } else if bps >= 1024.0 * 1024.0 {
        format!("{:.2} MB/s", bps / (1024.0 * 1024.0))
    } else if bps >= 1024.0 {
        format!("{:.1} KB/s", bps / 1024.0)
    } else {
        format!("{:.0} B/s", bps)
    }
}

pub fn format_eta(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

pub fn truncate_err(s: &str) -> String {
    let mut t = s.to_string();
    if t.len() > 240 {
        t.truncate(240);
        t.push('…');
    }
    t
}

pub fn folder_display_name(path: &Path, fallback: &str) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

pub const HOST_PALETTE: [&str; 8] = [
    "#007AFF", "#FF9500", "#34C759", "#AF52DE", "#FF2D55", "#5AC8FA", "#FFCC00", "#A2845E",
];

pub fn host_color(computer_id: Uuid, stored: Option<&str>) -> &'static str {
    if let Some(stored) = stored {
        for color in HOST_PALETTE {
            if stored.eq_ignore_ascii_case(color) {
                return color;
            }
        }
    }
    HOST_PALETTE[(computer_id.as_u128() as usize) % HOST_PALETTE.len()]
}

/// First palette entry that is not already stored on a host. Falls back to blue.
pub fn first_free_host_color<'a, I>(stored: I) -> &'static str
where
    I: IntoIterator<Item = &'a str>,
{
    let used: Vec<String> = stored
        .into_iter()
        .map(|color| color.to_ascii_lowercase())
        .collect();
    for color in HOST_PALETTE {
        if !used
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(color))
        {
            return color;
        }
    }
    HOST_PALETTE[0]
}

pub fn parse_port_field(raw: &str) -> Result<Option<u16>, ()> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match trimmed.parse::<u16>() {
        Ok(port) if port > 0 => Ok(Some(port)),
        _ => Err(()),
    }
}

/// Host part of an SSH destination or discovered hostname, for Bonjour matching.
pub fn ssh_host_key(destination: &str) -> String {
    let host = match destination.trim().rsplit_once('@') {
        Some((_, host)) => host.trim(),
        None => destination.trim(),
    };
    let host = host.trim_end_matches('.');
    let host = match host.strip_suffix(".local") {
        Some(stripped) => stripped,
        None => host,
    };
    host.to_ascii_lowercase()
}

fn port_key(port: Option<u16>) -> u16 {
    match port {
        None | Some(22) => 22,
        Some(port) => port,
    }
}

pub fn endpoints_match(
    dest_a: &str,
    port_a: Option<u16>,
    dest_b: &str,
    port_b: Option<u16>,
) -> bool {
    dest_a == dest_b && port_key(port_a) == port_key(port_b)
}

pub fn discovery_matches(
    bonjour_name: Option<&str>,
    ssh_destination: &str,
    ssh_port: Option<u16>,
    service_name: &str,
    discovered_host: &str,
    discovered_port: u16,
) -> bool {
    if bonjour_name.is_some_and(|name| name.eq_ignore_ascii_case(service_name)) {
        return true;
    }
    ssh_host_key(ssh_destination) == ssh_host_key(discovered_host)
        && port_key(ssh_port) == port_key(Some(discovered_port))
}

pub fn progress_fraction(progress: &ft_exec::Progress, transferring: bool) -> f32 {
    if let Some(pct) = progress.percent {
        (pct / 100.0).clamp(0.0, 1.0)
    } else {
        match (progress.bytes_done, progress.bytes_total) {
            (done, Some(total)) if total > 0 => (done as f32 / total as f32).clamp(0.0, 1.0),
            _ if transferring => -1.0,
            (done, _) if done > 0 => 1.0,
            _ => 0.0,
        }
    }
}

pub fn progress_detail(progress: &ft_exec::Progress, transferring: bool) -> String {
    let frac = progress_fraction(progress, transferring);
    let mut extras = Vec::new();
    match progress.files_total {
        Some(total) if total > 0 => extras.push(format!(
            "{} of {total} {}",
            progress.files_done.min(total),
            if total == 1 { "file" } else { "files" }
        )),
        _ if transferring && progress.files_done > 0 => extras.push(format!(
            "{} {}",
            progress.files_done,
            if progress.files_done == 1 {
                "file"
            } else {
                "files"
            }
        )),
        _ => {}
    }
    if let Some(rate) = progress.bytes_per_sec.filter(|r| *r > 0.0) {
        extras.push(format_rate(rate));
    }
    if let Some(eta) = progress.eta_secs {
        if transferring {
            extras.push(format!("ETA {}", format_eta(eta)));
        }
    }
    let suffix = if extras.is_empty() {
        String::new()
    } else {
        format!(" · {}", extras.join(" · "))
    };

    if frac < 0.0 {
        format!("{}{}", format_bytes(progress.bytes_done), suffix)
    } else {
        let pct_label = (frac * 100.0).round();
        match progress.bytes_total {
            Some(t) => format!(
                "{} / {} ({pct_label:.0}%){suffix}",
                format_bytes(progress.bytes_done),
                format_bytes(t),
            ),
            None => format!(
                "{} ({pct_label:.0}%){suffix}",
                format_bytes(progress.bytes_done)
            ),
        }
    }
}

pub fn status_kind(status_line: &str, transferring: bool) -> &'static str {
    if transferring {
        "busy"
    } else if status_line.contains("complete") || status_line.contains("Complete") {
        "ok"
    } else if status_line.contains("failed") || status_line.contains("Failed") {
        "err"
    } else {
        "idle"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_color_uses_stored_palette_or_hash() {
        let id = Uuid::nil();
        assert_eq!(host_color(id, Some("#ff9500")), "#FF9500");
        assert_eq!(host_color(id, Some("nope")), host_color(id, None));
    }

    #[test]
    fn first_free_skips_stored_colours() {
        assert_eq!(first_free_host_color(["#007AFF", "#ff9500"]), "#34C759");
        assert_eq!(first_free_host_color(HOST_PALETTE), "#007AFF");
    }

    #[test]
    fn port_field_accepts_empty_and_rejects_zero() {
        assert_eq!(parse_port_field("  "), Ok(None));
        assert_eq!(parse_port_field("22"), Ok(Some(22)));
        assert_eq!(parse_port_field("0"), Err(()));
        assert_eq!(parse_port_field("ssh"), Err(()));
    }

    #[test]
    fn ssh_host_key_strips_user_and_local() {
        assert_eq!(ssh_host_key("steve@studio.local"), "studio");
        assert_eq!(ssh_host_key("studio.local."), "studio");
        assert_eq!(ssh_host_key("studio"), "studio");
    }

    #[test]
    fn discovery_matches_bonjour_name_or_host_and_port() {
        assert!(discovery_matches(
            Some("office-nas"),
            "alias",
            None,
            "Office-NAS",
            "other.local",
            22
        ));
        assert!(discovery_matches(
            None,
            "steve@studio.local",
            None,
            "Studio",
            "studio.local",
            22
        ));
        assert!(discovery_matches(
            None,
            "steve@studio.local",
            Some(22),
            "x",
            "Studio.local.",
            22
        ));
        assert!(!discovery_matches(
            None,
            "steve@studio.local",
            Some(2222),
            "Studio",
            "studio.local",
            22
        ));
        assert!(endpoints_match("nas", None, "nas", Some(22)));
        assert!(!endpoints_match("nas", None, "other", Some(22)));
    }
}
