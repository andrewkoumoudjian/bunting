#![forbid(unsafe_code)]
//! `bunting-desk`: the trading workspace in one native window.

use bunting_desk::connect::{self, Options};
use bunting_desk::startup::StartupError;
use bunting_desk::{Desk, LayoutStore, Workspace};
use clap::Parser as _;
use futures_lite::StreamExt as _;
use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{
    App, AppContext as _, AssetSource, AsyncApp, Bounds, Result, SharedString, TitlebarOptions,
    WindowBounds, WindowOptions, px, size,
};
use std::borrow::Cow;

// The default component icon bundle plus the few catalog icons the desk
// uses, composed as in gpui-kit's examples (longbridge/gpui-kit,
// Apache-2.0, examples/speech/src/main.rs at 4d63c73).
icon_assets!(ExtraIcons, [Pencil, Trash]);

struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        Ok(paths)
    }
}

/// Quits the app through its normal path when the process is asked to stop
/// (SIGTERM or SIGINT on Unix, Ctrl+C on Windows), so the session logs out
/// and the local venue's claim drops instead of the venue running on.
fn quit_on_signals(cx: &mut App) {
    #[cfg(unix)]
    let wanted = [async_signal::Signal::Term, async_signal::Signal::Int];
    #[cfg(not(unix))]
    let wanted = [async_signal::Signal::Int];
    let mut signals = match async_signal::Signals::new(wanted) {
        Ok(signals) => signals,
        Err(error) => {
            eprintln!("bunting-desk: cannot watch for stop signals: {error}");
            return;
        }
    };
    cx.spawn(async move |cx: &mut AsyncApp| {
        if signals.next().await.is_some() {
            cx.update(|cx| cx.quit());
        }
    })
    .detach();
}

fn main() {
    let options = Options::parse();
    // Connect (or start the local venue) before opening a window, so a
    // wrong flag is reported on the terminal at once. Launched without a
    // terminal, the reason is shown in a window instead.
    let mut opened = Some(connect::open(&options).inspect_err(|error| {
        eprintln!("bunting-desk: {error}");
    }));
    let failed = matches!(opened, Some(Err(_)));
    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            bunting_desk::init(cx);
            quit_on_signals(cx);
            // One window per app: closing it quits, as on Linux and
            // Windows, instead of the macOS habit of running on with no
            // window (and still holding a participant in the local venue).
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let Some(opened) = opened.take() else { return };
            let titlebar = Some(TitlebarOptions {
                title: Some("Bunting".into()),
                ..TitlebarOptions::default()
            });
            let (venue, label) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    let bounds = Bounds::centered(None, size(px(520.), px(240.)), cx);
                    let window = WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        app_id: Some("bunting-desk".to_owned()),
                        titlebar,
                        ..WindowOptions::default()
                    };
                    if gpui_kit::open_window(window, cx, |_, cx| {
                        cx.new(|_| StartupError::new(&error))
                    })
                    .is_err()
                    {
                        cx.quit();
                    }
                    return;
                }
            };
            let desk = cx.new(|cx| {
                Desk::start_pump(cx);
                Desk::connected(venue, label.clone())
            });
            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
            let window = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // The Wayland app_id (window class) and title, so the
                // compositor can name and rule the window.
                app_id: Some("bunting-desk".to_owned()),
                titlebar,
                ..WindowOptions::default()
            };
            let store = LayoutStore::default_location();
            if let Err(error) = gpui_kit::open_window(window, cx, |window, cx| {
                cx.new(|cx| Workspace::new(desk.clone(), Some(store), window, cx))
            }) {
                eprintln!("bunting-desk: cannot open a window: {error}");
                cx.quit();
            }
        });
    if failed {
        std::process::exit(2);
    }
}
