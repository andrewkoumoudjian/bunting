#![forbid(unsafe_code)]

mod model;
mod terminal;

use gpui::{App, AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_component::TitleBar;
use gpui_component_assets::Assets;
use gpui_platform::application;
use terminal::Terminal;

fn main() {
    let app = application().with_assets(Assets);
    app.run(|cx: &mut App| {
        gpui_component::init(cx);

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
                let terminal = cx.new(|cx| Terminal::new(window, cx));
                terminal::root(terminal, window, cx)
            },
        )
        .expect("failed to open the Bunting terminal window");
        cx.activate(true);
    });
}
