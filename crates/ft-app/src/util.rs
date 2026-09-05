use std::path::Path;
use uuid::Uuid;

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

pub fn host_color(computer_id: Uuid) -> &'static str {
    const PALETTE: [&str; 8] = [
        "#007AFF", "#FF9500", "#34C759", "#AF52DE", "#FF2D55", "#5AC8FA", "#FFCC00", "#A2845E",
    ];
    PALETTE[(computer_id.as_u128() as usize) % PALETTE.len()]
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
