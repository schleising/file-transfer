use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Transfer,
    Computer,
    Folder,
    Document,
    Check,
    Close,
    Network,
    Home,
    Refresh,
    Plus,
    Help,
}

#[component]
pub fn Icon(kind: Glyph) -> Element {
    rsx! {
        svg {
            class: "glyph",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "1.8",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            {paths(kind)}
        }
    }
}

fn paths(kind: Glyph) -> Element {
    match kind {
        Glyph::Transfer => rsx! {
            path { d: "M7 7h9l-2.2-2.2M16 7l-2.2 2.2" }
            path { d: "M17 17H8l2.2 2.2M8 17l2.2-2.2" }
        },
        Glyph::Computer => rsx! {
            rect { x: "4", y: "5", width: "16", height: "11", rx: "2" }
            path { d: "M9 20h6M12 16v4" }
        },
        Glyph::Folder => rsx! {
            path { d: "M3.5 8.5A2 2 0 0 1 5.5 6.5h3.2l1.6 1.7h8.2a2 2 0 0 1 2 2V17a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" }
        },
        Glyph::Document => rsx! {
            path { d: "M7 4.5h7l4 4V19a1.5 1.5 0 0 1-1.5 1.5H7A1.5 1.5 0 0 1 5.5 19V6A1.5 1.5 0 0 1 7 4.5z" }
            path { d: "M14 4.5V9h4.5" }
            path { d: "M8.5 13h7M8.5 16.5h5" }
        },
        Glyph::Check => rsx! {
            path { d: "M5.5 12.5l4 4 9-9" }
        },
        Glyph::Close => rsx! {
            path { d: "M7 7l10 10M17 7L7 17" }
        },
        Glyph::Network => rsx! {
            circle { cx: "12", cy: "12", r: "1.6" }
            circle { cx: "12", cy: "5.2", r: "1.4" }
            circle { cx: "6.2", cy: "16.2", r: "1.4" }
            circle { cx: "17.8", cy: "16.2", r: "1.4" }
            path { d: "M12 10.4V6.8M10.7 13.1 7.3 15.4M13.3 13.1l3.4 2.3" }
        },
        Glyph::Home => rsx! {
            path { d: "M4.5 11.5 12 5l7.5 6.5" }
            path { d: "M7 10.5V19h10v-8.5" }
        },
        Glyph::Refresh => rsx! {
            path { d: "M19 12a7 7 0 1 1-2-4.9" }
            path { d: "M19 5v5h-5" }
        },
        Glyph::Plus => rsx! {
            path { d: "M12 5v14M5 12h14" }
        },
        Glyph::Help => rsx! {
            circle { cx: "12", cy: "12", r: "8" }
            path { d: "M9.1 9a3 3 0 0 1 5.8 1c0 2-3 2.4-3 4" }
            circle { cx: "12", cy: "17", r: "0.9", fill: "currentColor", stroke: "none" }
        },
    }
}
