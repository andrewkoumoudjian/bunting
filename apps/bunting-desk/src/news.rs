//! RIT's News module: headlines as the venue committed them, newest
//! first, and the selected item's text.

use crate::desk::Desk;
use crate::format;
use crate::panels::dock_panel;
use crate::ui::{self, Col};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::{ActiveTheme as _, IconName, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window, div,
    px, uniform_list,
};
use std::rc::Rc;

/// Width of the story beside the headlines: about 72 characters at the
/// body size, the longest line that stays easy to read.
const STORY_WIDTH: f32 = 468.;

const NEWS_COLUMNS: [Col; 3] = [
    Col::right("tick", 56.).priority(1),
    Col::left("symbol", 64.).priority(0),
    Col::left("headline", 160.).priority(2),
];

/// Why the Symbol column is empty.
const SYMBOL_GAP: &str = "BNP news items carry no symbol yet";

/// The list as tab-separated text with a header row, oldest first.
#[must_use]
pub fn copy_text(desk: &Desk) -> String {
    let header: Vec<&str> = NEWS_COLUMNS.iter().map(|col| col.label).collect();
    let body: Vec<Vec<String>> = desk
        .model()
        .news()
        .iter()
        .map(|item| {
            vec![
                tick(item.logical_time_ns),
                "—".to_owned(),
                item.headline.clone(),
            ]
        })
        .collect();
    ui::tsv(&header, &body)
}

/// RIT's Tick: whole seconds of run time.
fn tick(logical_time_ns: u64) -> String {
    format::grouped(i128::from(logical_time_ns / 1_000_000_000))
}

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
    #[expect(
        clippy::too_many_lines,
        reason = "the headline list and the story beside it read best together"
    )]
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let desk = self.desk.read(cx);
        let news = desk.model().news();
        let body = ui::pane_body("news", &self.focus);
        if news.is_empty() {
            return body.child(ui::empty_state(
                "news-empty",
                IconName::Info,
                "No news yet",
                None,
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
        let copy_desk = self.desk.clone();
        let story = shown;
        let layout = gpui_kit::container_query(move |size, _, cx| {
            // The story goes under the list in a tall window and beside it
            // in a wide one.
            let stacked = size.height > size.width;
            let desk = desk.clone();
            let select = select.clone();
            let headlines = v_flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .when(!stacked, gpui_kit::Styled::h_full)
                .child(ui::responsive_table(&NEWS_COLUMNS, move |fitted, _, cx| {
                    let fitted: Vec<ui::Fit> = fitted.to_vec();
                    v_flex()
                        .size_full()
                        .gap_1()
                        .child(ui::table_head(&fitted, cx))
                        .child(
                            uniform_list("news-rows", count, move |range, _, cx| {
                                let news = desk.read(cx).model().news();
                                let accent = cx.theme().secondary;
                                range
                                    .filter_map(|index| {
                                        news.get(news.len().checked_sub(index + 1)?)
                                    })
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
                                            .child(ui::table_row(
                                                &fitted,
                                                vec![
                                                    div()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child(tick(item.logical_time_ns))
                                                        .into_any_element(),
                                                    ui::tipped(
                                                        SharedString::from(format!(
                                                            "news-symbol-{id}"
                                                        )),
                                                        SYMBOL_GAP,
                                                        "—",
                                                    )
                                                    .into_any_element(),
                                                    div()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .child(item.headline.clone())
                                                        .into_any_element(),
                                                ],
                                                cx,
                                            ))
                                    })
                                    .collect()
                            })
                            .flex_1()
                            .min_h_0(),
                        )
                }));
            let mut columns = if stacked {
                v_flex().size_full().gap_3()
            } else {
                h_flex().size_full().gap_3()
            };
            columns = columns.child(headlines);
            if let Some(item) = story.clone() {
                let text = v_flex()
                    .id("news-text")
                    .test_support()
                    .overflow_y_scroll()
                    .gap_2()
                    .child(
                        div()
                            .text_size(ui::rem(ui::VALUE_SIZE))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(item.headline),
                    )
                    .child(ui::muted(
                        format!("run time {}", format::logical(item.logical_time_ns)),
                        cx,
                    ))
                    .child(
                        div()
                            .text_size(ui::rem(ui::VALUE_SIZE))
                            .line_height(gpui_kit::relative(1.5))
                            .child(item.body),
                    );
                columns = columns.child(if stacked {
                    text.flex_1()
                        .min_h_0()
                        .w_full()
                        .pt_2()
                        .border_t_1()
                        .border_color(cx.theme().border)
                } else {
                    text.flex_1()
                        .min_w_0()
                        .max_w(px(STORY_WIDTH))
                        .h_full()
                        .pl_3()
                        .border_l_1()
                        .border_color(cx.theme().border)
                });
            }
            columns
        })
        .w_full()
        .flex_1()
        .min_h_0();
        body.child(ui::copy_menu(
            v_flex()
                .id("news-table")
                .test_support()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(layout),
            copy_desk,
            Rc::new(copy_text),
        ))
    }
}
