//! macOS menubar extra, login-item, and launch visibility.

use dioxus::desktop::tao::event::{Event, WindowEvent};
use dioxus::desktop::trayicon::menu::{CheckMenuItem, Menu, PredefinedMenuItem};
use dioxus::desktop::trayicon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
use dioxus::desktop::{
    use_tray_icon_event_handler, use_tray_menu_event_handler, use_wry_event_handler,
};
use dioxus::prelude::*;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::msg_send;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const FOCUS_GRACE_MS: u64 = 400;

static WINDOW_FOCUSED: AtomicBool = AtomicBool::new(false);
static LAST_UNFOCUS_MS: AtomicU64 = AtomicU64::new(0);
static STARTUP_VISIBILITY_LOCKED: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
struct TrayHandles {
    _tray: TrayIcon,
}

pub fn attach_menubar() {
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
            STARTUP_VISIBILITY_LOCKED.store(true, Ordering::SeqCst);
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

    use_tray_menu_event_handler(move |event| {
        if event.id == *open_at_login.id() {
            let enable = !login_item_enabled();
            let _ = set_login_item(enable);
            open_at_login.set_checked(login_item_enabled());
        }
    });

    use_future(|| async {
        let hide = hide_window_at_launch();
        for _ in 0..16 {
            futures_timer::Delay::new(std::time::Duration::from_millis(40)).await;
            if STARTUP_VISIBILITY_LOCKED.load(Ordering::SeqCst) {
                break;
            }
            if hide {
                dioxus::desktop::window().window.set_visible(false);
            } else {
                show_window();
            }
        }
        STARTUP_VISIBILITY_LOCKED.store(true, Ordering::SeqCst);
    });
}

pub fn hide_window_at_launch() -> bool {
    if std::env::args().any(|a| a == "--hidden") {
        return true;
    }
    if launched_as_login_item_event() {
        return true;
    }
    running_from_app_bundle() && login_item_enabled() && !app_is_active()
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

const SM_ENABLED: isize = 1;

fn login_item_enabled() -> bool {
    unsafe {
        let Some(cls) = AnyClass::get(c"SMAppService") else {
            return false;
        };
        let svc: *mut AnyObject = msg_send![cls, mainAppService];
        if svc.is_null() {
            return false;
        }
        let status: isize = msg_send![svc, status];
        status == SM_ENABLED
    }
}

fn set_login_item(enabled: bool) -> bool {
    unsafe {
        let Some(cls) = AnyClass::get(c"SMAppService") else {
            return false;
        };
        let svc: *mut AnyObject = msg_send![cls, mainAppService];
        if svc.is_null() {
            return false;
        }
        let mut err: *mut AnyObject = std::ptr::null_mut();
        let ok: Bool = if enabled {
            msg_send![svc, registerAndReturnError: &mut err]
        } else {
            msg_send![svc, unregisterAndReturnError: &mut err]
        };
        ok.as_bool()
    }
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

#[link(name = "ServiceManagement", kind = "framework")]
extern "C" {}
