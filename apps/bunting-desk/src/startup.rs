//! The window shown when the app cannot reach or start its venue. A launch
//! from Finder, a desktop launcher or Explorer has no terminal, so the
//! reason has to be on screen.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    Context, FontWeight, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Window, div,
};

/// Why the desk could not open, with a way out.
pub struct StartupError {
    message: SharedString,
}

impl StartupError {
    #[must_use]
    pub fn new(message: &str) -> Self {
        // Errors read as clauses on a terminal; here the message is a sentence.
        let mut chars = message.chars();
        let message = chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect()
        });
        Self {
            message: message.into(),
        }
    }
}

impl Render for StartupError {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .p_6()
            .gap_3()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Bunting could not connect"),
            )
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(self.message.clone()),
            )
            .child(
                h_flex().justify_end().child(
                    Button::new("startup-quit")
                        .primary()
                        .label("Quit")
                        .on_click(|_, _, cx| cx.quit()),
                ),
            )
    }
}
