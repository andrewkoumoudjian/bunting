#![forbid(unsafe_code)]

mod local_server;
mod model;
mod shell;
mod terminal;

use gpui::{App, AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_component::{Theme, ThemeMode, TitleBar};
use gpui_component_assets::Assets;
use gpui_platform::application;
use shell::AppShell;

fn main() {
    let app = application().with_assets(Assets);
    app.run(|cx: &mut App| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        {
            let theme = Theme::global_mut(cx);
            theme.radius = px(4.);
            theme.radius_lg = px(6.);
            theme.tile_radius = px(0.);
            theme.tile_shadow = false;
            theme.shadow = false;
        }

        let bounds = Bounds::centered(None, size(px(1680.), px(980.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(1180.), px(720.))),
                titlebar: Some(TitleBar::title_bar_options()),
                app_id: Some("bunting-terminal".into()),
                ..Default::default()
            },
            |window, cx| {
                let shell = cx.new(|cx| AppShell::new(window, cx));
                shell::root(shell, window, cx)
            },
        )
        .expect("failed to open the Bunting terminal window");
        cx.activate(true);
    });
}
