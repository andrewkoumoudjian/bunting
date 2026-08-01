use crate::terminal::Terminal;
use gpui::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Window, div, px,
};
use gpui_component::{Root, TitleBar};

pub struct AppShell {
    terminal: Entity<Terminal>,
}

impl AppShell {
    pub fn new(terminal: Entity<Terminal>) -> Self {
        Self { terminal }
    }
}

impl Render for AppShell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                TitleBar::new()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("BUNTING MARKET TERMINAL"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .mr_3()
                            .text_xs()
                            .child("GPUI  •  FIXT.1.1 / FIX 5.0 SP2"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(self.terminal.clone()),
            )
    }
}

pub fn root(shell: Entity<AppShell>, window: &mut Window, cx: &mut App) -> Entity<Root> {
    cx.new(|cx| Root::new(shell, window, cx))
}
