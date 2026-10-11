//! The workspace renders every panel and the connection status in a real,
//! headless GPUI window (`gpui-kit` test support).
#![allow(clippy::expect_used, clippy::unwrap_used)]

use bunting_desk::{Connection, PANELS, Workspace};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, base::Root, px,
    size,
};

fn open(cx: &mut TestAppContext) -> (gpui_kit::WindowHandle<Root>, gpui_kit::Entity<Workspace>) {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1440.), px(900.)),
            })),
            ..WindowOptions::default()
        };
        let (window, workspace) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Workspace::new(window, cx))
        })
        .expect("open window");
        (window.downcast::<Root>().expect("Base Root"), workspace)
    })
}

#[gpui_kit::test]
fn every_panel_is_laid_out_and_the_first_tab_of_each_group_is_visible(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        for visible in ["books", "ticket", "tape", "orders"] {
            assert!(window.find(visible).visible(), "{visible} is visible");
        }
        assert!(PANELS.iter().all(|panel| !panel.purpose.is_empty()));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_status_bar_shows_the_connection_and_measured_round_trip(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("connection").visible());
        assert_eq!(
            workspace.read(cx).connection().label().as_ref(),
            "Not connected"
        );
        workspace.update(cx, |workspace, cx| {
            workspace.set_connection(
                Connection::Connected {
                    venue: "venue.example:9443".into(),
                    round_trip_us: Some(412),
                },
                cx,
            );
        });
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).connection().label().as_ref(),
            "Connected to venue.example:9443 · round trip 412 µs"
        );
    })
    .unwrap();
}
