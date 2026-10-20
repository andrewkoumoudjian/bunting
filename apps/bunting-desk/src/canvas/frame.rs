//! `WindowFrame`: the one title strip every canvas window wears (ADR 0051
//! decision 7): the pane's icon and title in the sans, its instrument chip
//! when it follows one, and the minimise, maximise and close actions on
//! the right. Panes draw no second title; this strip is the only one.
//! Dragging it moves the window; double-clicking maximises or restores.
//! The actions show only while the window is hovered or focused (rendered
//! at opacity 0 otherwise, so layout never shifts).

use super::{Canvas, TITLE_HEIGHT};
use crate::panes::PaneKind;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::{
    Context, FontWeight, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    SharedString, Styled as _, TestSupportExt as _, div, prelude::*, px,
};

/// The title strip of window `id`.
pub fn window_frame(
    id: u64,
    kind: PaneKind,
    instrument: Option<String>,
    focused: bool,
    maximised: bool,
    cx: &mut Context<Canvas>,
) -> impl IntoElement {
    let theme = cx.theme();
    // One foreground for every state in gpui-kit 0.7.1's custom variant,
    // so it matches the ghost buttons beside it (secondary foreground)
    // and stays legible on the danger hover ground.
    let close_style = ButtonCustomVariant::new(cx)
        .color(theme.transparent)
        .foreground(theme.secondary_foreground)
        .hover(theme.danger)
        .active(theme.danger_active);
    h_flex()
        .id(SharedString::from(format!("win-frame-{id}")))
        .test_support()
        .h(px(TITLE_HEIGHT))
        .flex_shrink_0()
        .w_full()
        .items_center()
        .gap_1()
        .px_2()
        .bg(theme.tab_bar)
        .border_b_1()
        .border_color(theme.border)
        .child(
            // The drag area: icon, title and chip, centred on the row.
            h_flex()
                .id(SharedString::from(format!("win-title-{id}")))
                .test_support()
                .flex_1()
                .min_w_0()
                .h_full()
                .items_center()
                .gap_2()
                .cursor_grab()
                .child(
                    Icon::new(kind.icon())
                        .small()
                        .text_color(theme.muted_foreground),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.foreground)
                        .truncate()
                        .child(kind.title()),
                )
                .when_some(instrument, |row, symbol| {
                    row.child(
                        h_flex()
                            .items_center()
                            .py(px(2.))
                            .px(px(6.))
                            .rounded(px(4.))
                            .bg(theme.muted)
                            .text_size(px(11.))
                            .text_color(theme.muted_foreground)
                            .child(symbol),
                    )
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event, window, cx| {
                        this.title_mouse_down(id, event, window, cx);
                    }),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .gap(px(2.))
                .opacity(if focused { 1. } else { 0. })
                .group_hover("canvas-window", |style| style.opacity(1.))
                .child(
                    frame_button(SharedString::from(format!("win-min-{id}")), IconName::Minus)
                        .on_click(cx.listener(move |this, _, _, cx| this.minimise(id, cx))),
                )
                .child(
                    frame_button(
                        SharedString::from(format!("win-max-{id}")),
                        if maximised {
                            IconName::Minimize
                        } else {
                            IconName::Maximize
                        },
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_maximise(id, cx))),
                )
                .child(
                    Button::new(SharedString::from(format!("win-close-{id}")))
                        .icon(IconName::Close)
                        .custom(close_style)
                        .xsmall()
                        .w(px(20.))
                        .h(px(20.))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.close_window(id, window, cx);
                        })),
                ),
        )
}

fn frame_button(id: SharedString, icon: IconName) -> Button {
    Button::new(id)
        .icon(icon)
        .ghost()
        .xsmall()
        .w(px(20.))
        .h(px(20.))
}
