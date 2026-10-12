//! RIT's News module: headlines as the venue committed them, newest
//! first, and the selected item's text.

use crate::desk::Desk;
use crate::format;
use crate::panels::{dock_panel, header, muted, panel_body, row};
use crate::theme;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, TestSupportExt as _, Window, div, px, uniform_list,
};
use std::rc::Rc;

/// Width of the story beside the headlines.
const STORY_WIDTH: f32 = 420.;

pub struct NewsPanel {
    desk: Entity<Desk>,
    focus: FocusHandle,
    /// The item whose text is shown; the latest when none is chosen.
    selected: Option<u128>,
    _observe: Subscription,
}

dock_panel!(NewsPanel, "news", "News");

impl NewsPanel {
    pub fn new(desk: Entity<Desk>, cx: &mut Context<Self>) -> Self {
        Self {
            _observe: cx.observe(&desk, |_, _, cx| cx.notify()),
            desk,
            focus: cx.focus_handle(),
            selected: None,
        }
    }
}

impl Render for NewsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let desk = self.desk.read(cx);
        let news = desk.model().news();
        let body = panel_body("news", &self.focus);
        if news.is_empty() {
            return body.child(muted(
                "Headlines appear here as the venue commits them, for everyone or for your team.",
                cx,
            ));
        }
        let shown = self
            .selected
            .and_then(|id| news.iter().find(|item| item.news_id == id))
            .or_else(|| news.back())
            .cloned();
        let selected = shown.as_ref().map(|item| item.news_id);
        let count = news.len();
        let select = Rc::new(cx.listener(|this, id: &u128, _, cx| {
            this.selected = Some(*id);
            cx.notify();
        }));
        let desk = self.desk.clone();
        let list = uniform_list("news-rows", count, move |range, _, cx| {
            let news = desk.read(cx).model().news();
            let accent = cx.theme().secondary;
            range
                .filter_map(|index| news.get(news.len().checked_sub(index + 1)?))
                .map(|item| {
                    let id = item.news_id;
                    let select = select.clone();
                    div()
                        .id(SharedString::from(format!("news-{id}")))
                        .test_support()
                        .cursor_pointer()
                        .rounded_sm()
                        .when(selected == Some(id), |line| line.bg(accent))
                        .on_click(move |_, window, cx| select(&id, window, cx))
                        .child(row(vec![
                            (
                                div()
                                    .child(format::logical(item.logical_time_ns))
                                    .into_any_element(),
                                72.,
                            ),
                            (div().child(item.headline.clone()).into_any_element(), 400.),
                        ]))
                })
                .collect()
        })
        .flex_1()
        .min_h_0();
        let headlines = v_flex()
            .flex_1()
            .min_w(px(320.))
            .h_full()
            .gap_1()
            .child(header(&[("run time", 72.), ("headline", 400.)], cx))
            .child(list);
        let mut columns = h_flex().flex_1().min_h_0().gap_3().child(headlines);
        if let Some(item) = shown {
            columns = columns.child(
                v_flex()
                    .id("news-text")
                    .test_support()
                    .w(px(STORY_WIDTH))
                    .flex_shrink_0()
                    .h_full()
                    .overflow_y_scroll()
                    .gap_1()
                    .pl_3()
                    .border_l_1()
                    .border_color(cx.theme().border)
                    .child(theme::serif(item.headline).text_size(px(17.)))
                    .child(muted(
                        format!("run time {}", format::logical(item.logical_time_ns)),
                        cx,
                    ))
                    .child(div().child(item.body)),
            );
        }
        body.child(columns)
    }
}
