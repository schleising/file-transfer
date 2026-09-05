//! Restore the last window size and position on launch.

use dioxus::desktop::tao::event::{Event, WindowEvent};
use dioxus::desktop::{LogicalPosition, LogicalSize, WindowBuilder};
use ft_store::Store;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const FRAME_KEY: &str = "window.frame";
const SAVE_DEBOUNCE_MS: u64 = 400;

const DEFAULT_WIDTH: f64 = 1280.0;
const DEFAULT_HEIGHT: f64 = 840.0;
const MIN_WIDTH: f64 = 900.0;
const MIN_HEIGHT: f64 = 560.0;

static LAST_SAVE_MS: AtomicU64 = AtomicU64::new(0);
static LAST_PERSISTED: Mutex<Option<String>> = Mutex::new(None);

#[derive(Clone, Copy, Debug)]
struct Frame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Frame {
    fn is_valid_size(self) -> bool {
        self.width.is_finite()
            && self.height.is_finite()
            && self.width >= MIN_WIDTH
            && self.height >= MIN_HEIGHT
            && self.width <= 4000.0
            && self.height <= 3000.0
    }

    fn has_position(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

pub fn apply(mut builder: WindowBuilder) -> WindowBuilder {
    builder = builder.with_min_inner_size(LogicalSize::new(MIN_WIDTH, MIN_HEIGHT));
    if let Some(frame) = load() {
        builder = builder.with_inner_size(LogicalSize::new(frame.width, frame.height));
        if frame.has_position() {
            builder = builder.with_position(LogicalPosition::new(frame.x, frame.y));
        }
    } else {
        builder = builder.with_inner_size(LogicalSize::new(DEFAULT_WIDTH, DEFAULT_HEIGHT));
    }
    builder
}

pub fn attach_persistence() {
    dioxus::desktop::use_wry_event_handler(|event, _| match event {
        Event::WindowEvent {
            event: WindowEvent::Moved(_) | WindowEvent::Resized(_),
            ..
        } => save_debounced(),
        Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        }
        | Event::LoopDestroyed => save(),
        _ => {}
    });
}

pub fn save() {
    LAST_SAVE_MS.store(now_ms(), Ordering::SeqCst);
    let Some(frame) = read_live_frame() else {
        return;
    };
    let _ = persist(frame);
}

fn save_debounced() {
    let now = now_ms();
    let last = LAST_SAVE_MS.load(Ordering::SeqCst);
    if now.saturating_sub(last) < SAVE_DEBOUNCE_MS {
        return;
    }
    save();
}

fn load() -> Option<Frame> {
    let raw = Store::open_default().ok()?.setting(FRAME_KEY).ok()??;
    parse_frame(&raw).filter(|frame| frame.is_valid_size())
}

fn persist(frame: Frame) -> anyhow::Result<()> {
    let value = format!("{} {} {} {}", frame.x, frame.y, frame.width, frame.height);
    if let Ok(mut last) = LAST_PERSISTED.lock() {
        if last.as_deref() == Some(value.as_str()) {
            return Ok(());
        }
        *last = Some(value.clone());
    }
    Store::open_default()?.set_setting(FRAME_KEY, &value)
}

fn read_live_frame() -> Option<Frame> {
    let window = &dioxus::desktop::window().window;
    if window.is_minimized() || window.fullscreen().is_some() {
        return None;
    }
    let scale = window.scale_factor();
    let size = window.inner_size().to_logical::<f64>(scale);
    let pos = window.outer_position().ok()?.to_logical::<f64>(scale);
    let frame = Frame {
        x: pos.x,
        y: pos.y,
        width: size.width.max(MIN_WIDTH),
        height: size.height.max(MIN_HEIGHT),
    };
    frame.is_valid_size().then_some(frame)
}

fn parse_frame(raw: &str) -> Option<Frame> {
    let mut parts = raw.split_whitespace();
    Some(Frame {
        x: parts.next()?.parse().ok()?,
        y: parts.next()?.parse().ok()?,
        width: parts.next()?.parse().ok()?,
        height: parts.next()?.parse().ok()?,
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
