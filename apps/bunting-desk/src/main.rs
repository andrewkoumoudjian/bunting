#![forbid(unsafe_code)]
//! `bunting-desk`: opens the trading workspace in one native window.

use bunting_desk::Workspace;
use gpui_kit::{AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..WindowOptions::default()
        };
        if let Err(error) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Workspace::new(window, cx))
        }) {
            eprintln!("bunting-desk: cannot open a window: {error}");
            cx.quit();
        }
    });
}
