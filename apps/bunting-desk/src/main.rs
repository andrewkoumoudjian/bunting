#![forbid(unsafe_code)]
//! `bunting-desk`: the trading workspace in one native window.

use bunting_desk::connect::{self, Options};
use bunting_desk::{Desk, Workspace};
use clap::Parser as _;
use gpui_kit::{AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};

fn main() {
    let options = Options::parse();
    // Connect (or start the local venue) before opening a window, so a
    // wrong flag is reported on the terminal at once.
    let (venue, label) = match connect::open(&options) {
        Ok(opened) => opened,
        Err(error) => {
            eprintln!("bunting-desk: {error}");
            std::process::exit(2);
        }
    };
    let mut venue = Some(venue);
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let Some(venue) = venue.take() else { return };
        let desk = cx.new(|cx| {
            Desk::start_pump(cx);
            Desk::connected(venue, label.clone())
        });
        let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
        let window = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..WindowOptions::default()
        };
        if let Err(error) = gpui_kit::open_window(window, cx, |window, cx| {
            cx.new(|cx| Workspace::new(desk.clone(), window, cx))
        }) {
            eprintln!("bunting-desk: cannot open a window: {error}");
            cx.quit();
        }
    });
}
