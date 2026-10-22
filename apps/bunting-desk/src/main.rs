#![forbid(unsafe_code)]
//! `bunting-desk`: the trading workspace in one native window.

use bunting_desk::connect::{self, Options};
use bunting_desk::onboarding::{Onboarding, default_choice_path};
use bunting_desk::startup::StartupError;
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
icon_assets!(ExtraIcons, [Pencil, Trash, Volume2, VolumeX]);

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
    // With no venue on the command line the app opens on the connect
    // screen (the RIT-style login): practise locally or join a hosted
    // venue. Flags connect at once, before a window opens, so a wrong one
    // is reported on the terminal; launched without a terminal, the
    // reason is shown in a window instead.
    let mut opened = options.names_a_venue().then(|| {
        connect::open(&options).inspect_err(|error| {
            eprintln!("bunting-desk: {error}");
        })
    });
    let failed = matches!(opened, Some(Err(_)));
    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            bunting_desk::init(cx);
            quit_on_signals(cx);
            // Closing the last window quits, as on Linux and Windows,
            // instead of the macOS habit of running on with no window (and
            // still holding a participant in the local venue).
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let small_window = |width: f32, height: f32, cx: &mut App| WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(width), px(height)),
                    cx,
                ))),
                app_id: Some("bunting-desk".to_owned()),
                titlebar: Some(TitlebarOptions {
                    title: Some("Bunting".into()),
                    ..TitlebarOptions::default()
                }),
                ..WindowOptions::default()
            };
            let Some(opened) = opened.take() else {
                let base = options.clone();
                let window = small_window(560., 560., cx);
                if gpui_kit::open_window(window, cx, |window, cx| {
                    cx.new(|cx| Onboarding::new(base, default_choice_path(), window, cx))
                })
                .is_err()
                {
                    cx.quit();
                }
                return;
            };
            match opened {
                Ok((venue, label)) => {
                    if let Err(error) = bunting_desk::open_desk_window(venue, label, cx) {
                        eprintln!("bunting-desk: {error}");
                        cx.quit();
                    }
                }
                Err(error) => {
                    let window = small_window(520., 240., cx);
                    if gpui_kit::open_window(window, cx, |_, cx| {
                        cx.new(|_| StartupError::new(&error))
                    })
                    .is_err()
                    {
                        cx.quit();
                    }
                }
            }
        });
    if failed {
        std::process::exit(2);
    }
}
