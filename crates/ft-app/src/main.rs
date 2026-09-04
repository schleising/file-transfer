mod icons;
mod state;
mod ui;
mod util;

fn main() {
    let window = {
        let builder = dioxus::desktop::WindowBuilder::new()
            .with_title("File Transfer")
            .with_inner_size(dioxus::desktop::LogicalSize::new(1120.0, 740.0))
            .with_min_inner_size(dioxus::desktop::LogicalSize::new(900.0, 560.0));
        #[cfg(target_os = "macos")]
        {
            use dioxus::desktop::tao::platform::macos::WindowBuilderExtMacOS;
            builder
                .with_titlebar_transparent(true)
                .with_fullsize_content_view(true)
                .with_title_hidden(true)
        }
        #[cfg(not(target_os = "macos"))]
        {
            builder
        }
    };

    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            dioxus::desktop::Config::new()
                .with_window(window)
                .with_menu(native_menu())
                .with_background_color((245, 245, 247, 255))
                .with_custom_head(format!(
                    "<style>{}</style>",
                    include_str!("../assets/macos.css")
                )),
        )
        .launch(ui::app);
}

fn native_menu() -> dioxus::desktop::muda::Menu {
    use dioxus::desktop::muda::{Menu, PredefinedMenuItem, Submenu};

    let menu = Menu::new();
    let app = Submenu::new("File Transfer", true);
    let _ = app.append_items(&[
        &PredefinedMenuItem::about(Some("About File Transfer"), None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::hide(None),
        &PredefinedMenuItem::hide_others(None),
        &PredefinedMenuItem::show_all(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::quit(None),
    ]);

    let edit = Submenu::new("Edit", true);
    let _ = edit.append_items(&[
        &PredefinedMenuItem::undo(None),
        &PredefinedMenuItem::redo(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::cut(None),
        &PredefinedMenuItem::copy(None),
        &PredefinedMenuItem::paste(None),
        &PredefinedMenuItem::select_all(None),
    ]);

    let window = Submenu::new("Window", true);
    let _ = window.append_items(&[
        &PredefinedMenuItem::minimize(None),
        &PredefinedMenuItem::maximize(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::fullscreen(None),
    ]);

    let _ = menu.append_items(&[&app, &edit, &window]);
    menu
}
