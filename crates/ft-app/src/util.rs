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

pub fn truncate_middle(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        return s.to_string();
    }
    if max_chars <= 1 {
        return "…".into();
    }
    let keep = max_chars - 1;
    let left = keep / 2;
    let right = keep - left;
    let mut out: String = chars[..left].iter().collect();
    out.push('…');
    out.extend(chars[chars.len() - right..].iter());
    out
}

pub fn folder_display_name(path: &std::path::Path, fallback: &str) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

pub fn host_color(computer_id: uuid::Uuid) -> egui::Color32 {
    const PALETTE: [egui::Color32; 8] = [
        egui::Color32::from_rgb(0, 122, 255),
        egui::Color32::from_rgb(255, 149, 0),
        egui::Color32::from_rgb(52, 199, 89),
        egui::Color32::from_rgb(175, 82, 222),
        egui::Color32::from_rgb(255, 45, 85),
        egui::Color32::from_rgb(90, 200, 250),
        egui::Color32::from_rgb(255, 204, 0),
        egui::Color32::from_rgb(162, 132, 94),
    ];
    let idx = (computer_id.as_u128() as usize) % PALETTE.len();
    PALETTE[idx]
}
