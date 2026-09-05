//! macOS menubar extra, login-item, and launch visibility.

use dioxus::desktop::tao::event::{Event, WindowEvent};
use dioxus::desktop::trayicon::menu::{CheckMenuItem, Menu, MenuId, PredefinedMenuItem};
use dioxus::desktop::trayicon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
use dioxus::desktop::{
    use_muda_event_handler, use_tray_icon_event_handler, use_tray_menu_event_handler,
    use_wry_event_handler,
};
use dioxus::prelude::*;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const AGENT_LABEL: &str = "local.file-transfer";
const INSTALLED_APP: &str = "/Applications/File Transfer.app";

unsafe extern "C" {
    fn getuid() -> u32;
}

const FOCUS_GRACE_MS: u64 = 400;

static WINDOW_FOCUSED: AtomicBool = AtomicBool::new(false);
static LAST_UNFOCUS_MS: AtomicU64 = AtomicU64::new(0);
static STARTUP_VISIBILITY_LOCKED: AtomicBool = AtomicBool::new(false);
static STARTUP_REOPEN: AtomicBool = AtomicBool::new(false);
static PROCESS_START_MS: AtomicU64 = AtomicU64::new(0);

const STARTUP_REOPEN_GRACE_MS: u64 = 2500;

#[derive(Clone)]
struct TrayHandles {
    _tray: TrayIcon,
}

pub fn attach_menubar() {
    let _ = PROCESS_START_MS.compare_exchange(0, now_ms(), Ordering::SeqCst, Ordering::SeqCst);
    let open_at_login = use_hook(|| {
        let open_at_login = CheckMenuItem::new("Open at Login", true, login_item_enabled(), None);
        let menu = Menu::new();
        let _ = menu.append_items(&[
            &open_at_login,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ]);
        let tray = TrayIconBuilder::new()
            .with_tooltip("File Transfer")
            .with_icon(menubar_icon())
            .with_icon_as_template(true)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .expect("menubar extra");
        provide_context(TrayHandles { _tray: tray });
        Rc::new(open_at_login)
    });

    use_wry_event_handler(|event, _| match event {
        Event::WindowEvent {
            event: WindowEvent::Focused(focused),
            ..
        } => note_focus(*focused),
        Event::Reopen { .. } => {
            if !STARTUP_VISIBILITY_LOCKED.load(Ordering::SeqCst) {
                STARTUP_REOPEN.store(true, Ordering::SeqCst);
                dioxus::desktop::window().window.set_visible(false);
                return;
            }
            let too_soon =
                now_ms().saturating_sub(PROCESS_START_MS.load(Ordering::SeqCst))
                    < STARTUP_REOPEN_GRACE_MS;
            if too_soon && STARTUP_REOPEN.load(Ordering::SeqCst) {
                dioxus::desktop::window().window.set_visible(false);
                return;
            }
            show_window();
        }
        _ => {}
    });

    use_tray_icon_event_handler(|event| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Down,
            ..
        } = event
        {
            toggle_window();
        }
    });

    let login_tray = open_at_login.clone();
    use_tray_menu_event_handler(move |event| {
        apply_open_at_login(&login_tray, &event.id);
    });
    let login_muda = open_at_login.clone();
    use_muda_event_handler(move |event| {
        apply_open_at_login(&login_muda, &event.id);
    });

    use_future(|| async {
        for _ in 0..16 {
            futures_timer::Delay::new(std::time::Duration::from_millis(40)).await;
            if STARTUP_VISIBILITY_LOCKED.load(Ordering::SeqCst) {
                break;
            }
            if STARTUP_REOPEN.load(Ordering::SeqCst) || hide_window_at_launch() {
                dioxus::desktop::window().window.set_visible(false);
            } else {
                show_window();
            }
        }
        STARTUP_VISIBILITY_LOCKED.store(true, Ordering::SeqCst);
    });
}

fn apply_open_at_login(item: &CheckMenuItem, event_id: &MenuId) {
    if event_id != item.id() {
        return;
    }
    let enable = !login_item_enabled();
    let _ = set_login_item(enable);
    item.set_checked(login_item_enabled());
}

pub fn hide_window_at_launch() -> bool {
    if std::env::args().any(|a| a == "--hidden") {
        return true;
    }
    if launched_as_login_item_event() {
        return true;
    }
    running_from_app_bundle() && !app_is_active()
}

fn toggle_window() {
    STARTUP_VISIBILITY_LOCKED.store(true, Ordering::SeqCst);
    let desktop = dioxus::desktop::window();
    if !desktop.window.is_visible() {
        show_window();
        return;
    }
    if desktop.window.is_focused() || was_front_recently() {
        desktop.window.set_visible(false);
    } else {
        show_window();
    }
}

fn show_window() {
    let desktop = dioxus::desktop::window();
    desktop.window.set_visible(true);
    desktop.window.set_focus();
    activate_app();
    note_focus(true);
}

fn note_focus(focused: bool) {
    WINDOW_FOCUSED.store(focused, Ordering::SeqCst);
    if !focused {
        LAST_UNFOCUS_MS.store(now_ms(), Ordering::SeqCst);
    }
}

fn was_front_recently() -> bool {
    WINDOW_FOCUSED.load(Ordering::SeqCst)
        || now_ms().saturating_sub(LAST_UNFOCUS_MS.load(Ordering::SeqCst)) < FOCUS_GRACE_MS
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn running_from_app_bundle() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.contains(".app/Contents/MacOS/")))
        .unwrap_or(false)
}

fn launched_as_login_item_event() -> bool {
    unsafe {
        let Some(cls) = AnyClass::get(c"NSAppleEventManager") else {
            return false;
        };
        let manager: *mut AnyObject = msg_send![cls, sharedAppleEventManager];
        if manager.is_null() {
            return false;
        }
        let event: *mut AnyObject = msg_send![manager, currentAppleEvent];
        if event.is_null() {
            return false;
        }
        let event_id: u32 = msg_send![event, eventID];
        const OPEN_APP: u32 = u32::from_be_bytes(*b"oapp");
        const LOGIN: u32 = u32::from_be_bytes(*b"lgit");
        const SERVICE: u32 = u32::from_be_bytes(*b"svit");
        if event_id != OPEN_APP {
            return false;
        }
        let login: *mut AnyObject = msg_send![event, paramDescriptorForKeyword: LOGIN];
        let service: *mut AnyObject = msg_send![event, paramDescriptorForKeyword: SERVICE];
        !login.is_null() || !service.is_null()
    }
}

fn app_is_active() -> bool {
    unsafe {
        let Some(cls) = AnyClass::get(c"NSRunningApplication") else {
            return false;
        };
        let app: *mut AnyObject = msg_send![cls, currentApplication];
        if app.is_null() {
            return false;
        }
        let active: Bool = msg_send![app, isActive];
        active.as_bool()
    }
}

fn activate_app() {
    unsafe {
        let Some(cls) = AnyClass::get(c"NSApplication") else {
            return;
        };
        let app: *mut AnyObject = msg_send![cls, sharedApplication];
        if app.is_null() {
            return;
        }
        let _: () = msg_send![app, activateIgnoringOtherApps: Bool::YES];
    }
}

fn login_item_enabled() -> bool {
    agent_plist_path().is_some_and(|p| p.is_file())
}

fn set_login_item(enabled: bool) -> bool {
    let Some(plist) = agent_plist_path() else {
        return false;
    };
    bootout_agent();
    if !enabled {
        let _ = std::fs::remove_file(&plist);
        return !plist.exists();
    }
    let Some(app) = app_bundle_path() else {
        return false;
    };
    if let Some(dir) = plist.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
    }
    if std::fs::write(&plist, launch_agent_plist(&app)).is_err() {
        return false;
    }
    if !bootstrap_agent(&plist) {
        let _ = std::fs::remove_file(&plist);
        return false;
    }
    true
}

fn agent_plist_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join("Library/LaunchAgents").join(format!("{AGENT_LABEL}.plist")))
}

fn app_bundle_path() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(app) = bundle_from_exe(&exe) {
            return Some(app);
        }
    }
    let installed = PathBuf::from(INSTALLED_APP);
    installed.is_dir().then_some(installed)
}

fn bundle_from_exe(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    if macos.file_name()?.to_str()? != "MacOS" {
        return None;
    }
    let app = macos.parent()?.parent()?;
    (app.extension()?.to_str()? == "app").then(|| app.to_path_buf())
}

fn launch_agent_plist(app: &Path) -> String {
    let app = xml_escape(&app.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{AGENT_LABEL}</string>
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/open</string>
    <string>-g</string>
    <string>-a</string>
    <string>{app}</string>
    <string>--args</string>
    <string>--hidden</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn session_uid() -> u32 {
    unsafe { getuid() }
}

fn agent_target() -> String {
    format!("gui/{}/{AGENT_LABEL}", session_uid())
}

fn bootout_agent() {
    let _ = Command::new("/bin/launchctl")
        .args(["bootout", &agent_target()])
        .output();
}

fn bootstrap_agent(plist: &Path) -> bool {
    let domain = format!("gui/{}", session_uid());
    let path = plist.to_string_lossy();
    let _ = Command::new("/bin/launchctl")
        .args(["enable", &agent_target()])
        .status();
    let boot = Command::new("/bin/launchctl")
        .args(["bootstrap", &domain, path.as_ref()])
        .output();
    if let Ok(out) = boot {
        if out.status.success() {
            return true;
        }
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("already loaded") || err.contains("service already") {
            return true;
        }
    }
    Command::new("/bin/launchctl")
        .args(["load", "-w", path.as_ref()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn menubar_icon() -> Icon {
    const S: u32 = 44;
    let mut px = vec![0u8; (S * S * 4) as usize];
    draw_line(&mut px, S, 8, 14, 32, 14);
    draw_line(&mut px, S, 24, 8, 32, 14);
    draw_line(&mut px, S, 24, 20, 32, 14);
    draw_line(&mut px, S, 36, 30, 12, 30);
    draw_line(&mut px, S, 20, 24, 12, 30);
    draw_line(&mut px, S, 20, 36, 12, 30);
    Icon::from_rgba(px, S, S).expect("menubar icon")
}

fn draw_line(px: &mut [u8], s: u32, x0: i32, y0: i32, x1: i32, y1: i32) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut x = x0;
    let mut y = y0;
    loop {
        stamp(px, s, x, y);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

fn stamp(px: &mut [u8], s: u32, x: i32, y: i32) {
    for dy in -1..=1 {
        for dx in -1..=1 {
            let xx = x + dx;
            let yy = y + dy;
            if xx < 0 || yy < 0 || xx >= s as i32 || yy >= s as i32 {
                continue;
            }
            let i = ((yy as u32 * s + xx as u32) * 4) as usize;
            px[i] = 0;
            px[i + 1] = 0;
            px[i + 2] = 0;
            px[i + 3] = 255;
        }
    }
}

