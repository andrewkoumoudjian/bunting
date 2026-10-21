//! The canvas of floating windows (ADR 0051 decisions 2 and 7): each pane
//! lives in a window that can be moved by its title strip, resized from
//! any edge, minimised to the sidebar, maximised within the canvas and
//! closed (and reopened). Windows snap to the canvas and to each other
//! while dragging and show the guide they snapped to. Pure geometry lives
//! in [`geometry`]; the one title strip is in [`frame`].
//!
//! Every edit emits [`CanvasEvent::Changed`], which the workspace turns
//! into the debounced screen save. Rendering clamps windows into a shrunken
//! canvas without rewriting their saved rects.
#![allow(
    clippy::float_arithmetic,
    reason = "drag and placement math is f32 logical pixels"
)]

pub mod frame;
#[allow(
    clippy::float_arithmetic,
    reason = "window geometry is f32 logical pixels throughout"
)]
pub mod geometry;

use crate::desk::Desk;
use crate::panes::PaneKind;
use crate::theme;
use geometry::{Edge, GUTTER, Guide, Point, Rect};
use gpui_kit::component::button::Button;
use gpui_kit::component::dock::BasePanelView;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, Div, Entity, EventEmitter, FontWeight, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _,
    Pixels, Point as GpuiPoint, Render, SharedString, Styled as _, TestSupportExt as _, Window,
    canvas as paint_canvas, div, prelude::*, px,
};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

/// The title strip's height: [`frame::WindowFrame`] and the clamp agree.
pub const TITLE_HEIGHT: f32 = 28.;
/// How close an edge must come to snap.
const SNAP_THRESHOLD: f32 = 8.;
/// Cascade offset between newly opened windows.
const CASCADE_STEP: f32 = 28.;
/// Closed windows kept for reopen.
const MAX_CLOSED: usize = 16;

/// One window's saved form: rebuilt with `PaneKind::from_name` + `build`.
/// Panes dump no custom state, so kind and geometry are the whole record.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SavedWindow {
    pub kind: String,
    pub rect: Rect,
    pub z: usize,
    pub minimised: bool,
    pub maximised: bool,
}

/// What the sidebar's Windows list shows about one window.
#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub id: u64,
    pub kind: PaneKind,
    pub minimised: bool,
    pub focused: bool,
}

/// Canvas edits the workspace saves (debounced) and redraws for.
pub enum CanvasEvent {
    Changed,
}

impl EventEmitter<CanvasEvent> for Canvas {}

struct CanvasWindow {
    id: u64,
    kind: PaneKind,
    view: Arc<dyn BasePanelView>,
    /// The rect at the current painted canvas size.
    rect: Rect,
    /// The rect as last set by the user, in the base layout's canvas
    /// size. Reflow always derives `rect` from this, so resizing forth
    /// and back never drifts the layout.
    base_rect: Rect,
    /// The floating rect to restore from maximise.
    restore: Option<Rect>,
    minimised: bool,
    maximised: bool,
}

struct ClosedWindow {
    kind: PaneKind,
    rect: Rect,
}

#[derive(Clone, Copy)]
enum DragKind {
    Move,
    Resize(Edge),
}

struct Drag {
    id: u64,
    kind: DragKind,
    /// Where the drag began, in canvas coordinates.
    start: Point,
    start_rect: Rect,
}

/// The floating windows of the shown screen.
pub struct Canvas {
    desk: Entity<Desk>,
    /// Back to front; the last is on top.
    windows: Vec<CanvasWindow>,
    closed: Vec<ClosedWindow>,
    focused: Option<u64>,
    next_id: u64,
    screen_name: String,
    /// The screen's template as canvas fractions, for the empty state.
    template: Vec<(PaneKind, Rect)>,
    /// The template has not been placed yet: the canvas has not painted,
    /// so its real size is unknown and fractions must not be placed (or
    /// saved) against the seed size.
    pending_template: bool,
    /// The canvas size the base layout was laid out for. When the
    /// painted size differs, windows reflow from the base (a screen
    /// saved on a desktop monitor shown on a laptop); only user edits
    /// rewrite the base. `None` adopts the first painted size.
    base_size: Option<(f32, f32)>,
    /// Last known canvas size; seeded by the workspace, kept by paint.
    size: (f32, f32),
    /// The canvas bounds in window coordinates, recorded each paint.
    bounds: Rc<Cell<Rect>>,
    drag: Option<Drag>,
    guides: Vec<Guide>,
}

impl Canvas {
    #[must_use]
    pub fn new(desk: Entity<Desk>, size: (f32, f32)) -> Self {
        Self {
            desk,
            windows: Vec::new(),
            closed: Vec::new(),
            focused: None,
            next_id: 1,
            screen_name: String::new(),
            template: Vec::new(),
            pending_template: false,
            base_size: None,
            size,
            bounds: Rc::new(Cell::new(Rect::default())),
            drag: None,
            guides: Vec::new(),
        }
    }

    #[must_use]
    pub const fn size(&self) -> (f32, f32) {
        self.size
    }

    /// Whether a template is waiting for the first painted bounds.
    #[must_use]
    pub const fn has_pending_template(&self) -> bool {
        self.pending_template
    }

    /// The canvas size the base layout was laid out for, once known.
    #[must_use]
    pub const fn base_size(&self) -> Option<(f32, f32)> {
        self.base_size
    }

    fn canvas_rect(&self) -> Rect {
        Rect::new(0., 0., self.size.0, self.size.1)
    }

    /// Shows a screen: its saved windows (laid out for `layout_size`,
    /// reflowed when the painted canvas differs), or its template
    /// (`fractions` of the canvas) when nothing is saved yet. A template
    /// waits for the first painted bounds so it is never placed against
    /// a seed size.
    pub fn set_screen(
        &mut self,
        name: String,
        fractions: &[(PaneKind, Rect)],
        saved: Option<&[SavedWindow]>,
        layout_size: Option<(f32, f32)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.screen_name = name;
        self.template = fractions.to_vec();
        self.base_size = layout_size;
        self.pending_template = false;
        self.windows.clear();
        self.closed.clear();
        self.focused = None;
        self.drag = None;
        self.guides.clear();
        match saved {
            Some(saved) => {
                let mut ordered: Vec<&SavedWindow> = saved.iter().collect();
                ordered.sort_by_key(|each| each.z);
                for each in ordered {
                    if let Some(kind) = PaneKind::from_name(&each.kind) {
                        let id = self.push(kind, each.rect, window, cx);
                        if let Some(win) = self.window_mut(id) {
                            win.minimised = each.minimised;
                            win.maximised = each.maximised;
                            if each.maximised {
                                win.restore = Some(each.rect);
                            }
                        }
                    }
                }
            }
            None => {
                if self.bounds.get().w > 0. {
                    self.place_template(fractions, window, cx);
                } else {
                    self.pending_template = true;
                }
            }
        }
        self.focused = self
            .windows
            .iter()
            .rev()
            .find(|win| !win.minimised)
            .map(|win| win.id);
        cx.notify();
    }

    /// Every window in saved form, back to front.
    #[must_use]
    pub fn dump(&self) -> Vec<SavedWindow> {
        self.windows
            .iter()
            .enumerate()
            .map(|(z, win)| SavedWindow {
                kind: win.kind.name().to_owned(),
                rect: win.base_rect,
                z,
                minimised: win.minimised,
                maximised: win.maximised,
            })
            .collect()
    }

    /// The windows for the sidebar list, front first.
    #[must_use]
    pub fn window_list(&self) -> Vec<WindowInfo> {
        self.windows
            .iter()
            .rev()
            .map(|win| WindowInfo {
                id: win.id,
                kind: win.kind,
                minimised: win.minimised,
                focused: Some(win.id) == self.focused && !win.minimised,
            })
            .collect()
    }

    // ---- opening and closing -------------------------------------------

    fn push(
        &mut self,
        kind: PaneKind,
        rect: Rect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let view = kind.build(self.desk.clone(), window, cx);
        self.windows.push(CanvasWindow {
            id,
            kind,
            view,
            rect,
            base_rect: rect,
            restore: None,
            minimised: false,
            maximised: false,
        });
        id
    }

    /// Opens a window of `kind`, cascaded from the focused one (or from the
    /// top-left), at the kind's default size and fully within the canvas
    /// inset by the gutter.
    pub fn open(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let canvas = self.canvas_rect().inset(GUTTER);
        let (min_w, min_h) = kind.min_size();
        let (def_w, def_h) = kind.default_size();
        let w = def_w.min(canvas.w).max(min_w.min(canvas.w));
        let h = def_h.min(canvas.h).max(min_h.min(canvas.h));
        let base = self
            .focused
            .and_then(|id| self.window(id))
            .filter(|win| !win.minimised)
            .map_or((canvas.x, canvas.y), |win| {
                let rect = self.display_rect(win);
                (rect.x + CASCADE_STEP, rect.y + CASCADE_STEP)
            });
        let x = base.0.clamp(canvas.x, (canvas.right() - w).max(canvas.x));
        let y = base.1.clamp(canvas.y, (canvas.bottom() - h).max(canvas.y));
        let id = self.push(kind, Rect::new(x, y, w, h), window, cx);
        self.focus_window(id, window, cx);
        self.rebase();
        self.changed(cx);
        id
    }

    /// Opens another window of window `id`'s kind next to it: to its
    /// right when there is room, else cascaded from it.
    pub fn open_beside(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.window(id) else {
            return;
        };
        let kind = source.kind;
        let from = self.display_rect(source);
        let canvas = self.canvas_rect().inset(GUTTER);
        let (def_w, def_h) = kind.default_size();
        let (min_w, min_h) = kind.min_size();
        let w = def_w.min(from.w).max(min_w).min(canvas.w);
        let h = from.h.max(min_h).min(canvas.h);
        let _ = def_h;
        let x = from.right() + GUTTER;
        if x + w <= canvas.right() {
            let rect = Rect::new(x, from.y, w, h);
            let new = self.push(kind, rect, window, cx);
            self.focus_window(new, window, cx);
            self.rebase();
            self.changed(cx);
        } else {
            self.focus_window(id, window, cx);
            self.open(kind, window, cx);
        }
    }

    /// Closes every window. Each goes onto the bounded reopen stack, so
    /// Reopen closed window brings the last ones back.
    pub fn close_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self.windows.iter().map(|win| win.id).collect();
        for id in ids {
            self.close_window(id, window, cx);
        }
    }

    /// How many windows are open.
    #[must_use]
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    /// Places fractional template windows on the current canvas and
    /// reports the edit.
    pub fn place_fractions(
        &mut self,
        fractions: &[(PaneKind, Rect)],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.place_template(fractions, window, cx);
        self.changed(cx);
    }

    /// Places the screen's own template and reports the edit.
    pub fn apply_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let template = self.template.clone();
        self.place_fractions(&template, window, cx);
    }

    fn place_template(
        &mut self,
        fractions: &[(PaneKind, Rect)],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_template = false;
        self.base_size = Some(self.size);
        let canvas = self.canvas_rect();
        for &(kind, fraction) in fractions {
            let rect = geometry::place_fraction(fraction, canvas);
            self.push(kind, rect, window, cx);
        }
        self.focused = self.windows.last().map(|win| win.id);
        cx.notify();
    }

    /// Closes the focused window.
    pub fn close_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.focused {
            self.close_window(id, window, cx);
        }
    }

    /// Closes a window, keeping it on the bounded reopen stack. When the
    /// focused window goes, keyboard focus moves to the next window so
    /// key commands keep working.
    pub fn close_window(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.windows.iter().position(|win| win.id == id) else {
            return;
        };
        let win = self.windows.remove(index);
        self.closed.push(ClosedWindow {
            kind: win.kind,
            rect: if win.maximised {
                win.restore.unwrap_or(win.rect)
            } else {
                win.rect
            },
        });
        if self.closed.len() > MAX_CLOSED {
            self.closed.remove(0);
        }
        if self.focused == Some(id) {
            self.focused = self
                .windows
                .iter()
                .rev()
                .find(|win| !win.minimised)
                .map(|win| win.id);
            if let Some(next) = self.focused.and_then(|next| self.window(next)) {
                let handle = next.view.focus_handle(cx);
                window.focus(&handle, cx);
            }
        }
        self.rebase();
        self.changed(cx);
    }

    /// Reopens the last closed window with its geometry.
    pub fn reopen_closed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(closed) = self.closed.pop() else {
            return;
        };
        // Closed rects are kept in base space; bring this one to the
        // painted size before it re-enters the base.
        let live = match self.base_size {
            Some(base) if base != self.size => geometry::reflow(
                &[closed.rect],
                Rect::new(0., 0., base.0, base.1),
                self.canvas_rect(),
                &[closed.kind.min_size()],
            )[0],
            _ => closed.rect,
        };
        let rect = geometry::clamp_to_canvas(live, self.canvas_rect(), TITLE_HEIGHT);
        let id = self.push(closed.kind, rect, window, cx);
        self.focus_window(id, window, cx);
        self.rebase();
        self.changed(cx);
    }

    // ---- arrangement -----------------------------------------------------

    /// Tiles the visible windows in a grid filling the canvas.
    pub fn tile_windows(&mut self, cx: &mut Context<Self>) {
        let visible: Vec<u64> = self
            .windows
            .iter()
            .filter(|win| !win.minimised)
            .map(|win| win.id)
            .collect();
        let rects = geometry::tile(visible.len(), self.canvas_rect(), GUTTER);
        for (id, rect) in visible.into_iter().zip(rects) {
            if let Some(win) = self.window_mut(id) {
                win.rect = rect;
                win.maximised = false;
                win.restore = None;
            }
        }
        self.rebase();
        self.changed(cx);
    }

    /// Fans the visible windows out from the top-left, each at least its
    /// minimum size.
    pub fn cascade_windows(&mut self, cx: &mut Context<Self>) {
        let visible: Vec<u64> = self
            .windows
            .iter()
            .filter(|win| !win.minimised)
            .map(|win| win.id)
            .collect();
        let rects = geometry::cascade(visible.len(), self.canvas_rect(), CASCADE_STEP);
        for (id, rect) in visible.into_iter().zip(rects) {
            if let Some(win) = self.window_mut(id) {
                win.rect = geometry::enforce_min(rect, win.kind.min_size());
                win.maximised = false;
                win.restore = None;
            }
        }
        self.rebase();
        self.changed(cx);
    }

    /// Focuses the next or previous visible window, wrapping around.
    pub fn cycle_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let visible: Vec<u64> = self
            .windows
            .iter()
            .filter(|win| !win.minimised)
            .map(|win| win.id)
            .collect();
        if visible.is_empty() {
            return;
        }
        let current = self
            .focused
            .and_then(|id| visible.iter().position(|each| *each == id));
        let next = match current {
            Some(index) if forward => visible[(index + 1) % visible.len()],
            Some(index) => visible[(index + visible.len() - 1) % visible.len()],
            None if forward => visible[0],
            None => visible[visible.len() - 1],
        };
        self.focus_window(next, window, cx);
        cx.notify();
    }

    /// Maximises the focused window, or restores it.
    pub fn maximise_focused(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.focused {
            self.toggle_maximise(id, cx);
        }
    }

    /// Maximises a window within the canvas, or restores its floating rect.
    pub fn toggle_maximise(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(win) = self.window_mut(id) else {
            return;
        };
        if win.maximised {
            win.maximised = false;
            if let Some(restore) = win.restore.take() {
                win.rect = restore;
            }
        } else {
            win.restore = Some(win.rect);
            win.maximised = true;
            win.minimised = false;
        }
        self.bring_to_front(id);
        self.focused = Some(id);
        self.changed(cx);
    }

    /// Minimises a window to the sidebar's Windows list.
    pub fn minimise(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(win) = self.window_mut(id) else {
            return;
        };
        win.minimised = true;
        if self.focused == Some(id) {
            self.focused = self
                .windows
                .iter()
                .rev()
                .find(|win| !win.minimised)
                .map(|win| win.id);
        }
        self.changed(cx);
    }

    /// Brings a window to the front, restores it if minimised, and gives it
    /// keyboard focus.
    pub fn focus_window(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(win) = self.window_mut(id) else {
            return;
        };
        win.minimised = false;
        self.bring_to_front(id);
        self.focused = Some(id);
        if let Some(win) = self.window(id) {
            let handle = win.view.focus_handle(cx);
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    fn bring_to_front(&mut self, id: u64) {
        if let Some(index) = self.windows.iter().position(|win| win.id == id)
            && index + 1 != self.windows.len()
        {
            let win = self.windows.remove(index);
            self.windows.push(win);
        }
    }

    fn window(&self, id: u64) -> Option<&CanvasWindow> {
        self.windows.iter().find(|win| win.id == id)
    }

    fn window_mut(&mut self, id: u64) -> Option<&mut CanvasWindow> {
        self.windows.iter_mut().find(|win| win.id == id)
    }

    #[allow(
        clippy::unused_self,
        reason = "reads as the canvas reporting its own edit"
    )]
    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(CanvasEvent::Changed);
        cx.notify();
    }

    /// Adopts the painted canvas size: places a pending template against
    /// it, or reflows laid-out windows when it differs from the size they
    /// were laid out for (startup on another display, window resize).
    fn settle_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let painted = self.bounds.get();
        if painted.w <= 0. || painted.h <= 0. {
            if self.pending_template {
                // Ask for the frame after the first paint records bounds.
                cx.notify();
            }
            return;
        }
        let new_size = (painted.w, painted.h);
        // Reflow only on an actual resize, never per frame: between
        // resizes the live rects are the user's to drag.
        let resized =
            (new_size.0 - self.size.0).abs() > 0.5 || (new_size.1 - self.size.1).abs() > 0.5;
        self.size = new_size;
        if self.pending_template {
            let template = self.template.clone();
            self.place_template(&template, window, cx);
            return;
        }
        match self.base_size {
            None => {
                // Adopt the painted size as the base: rects just loaded
                // or seeded are taken to be laid out for it.
                self.base_size = Some(new_size);
                for win in &mut self.windows {
                    win.base_rect = if win.maximised {
                        win.restore.unwrap_or(win.rect)
                    } else {
                        win.rect
                    };
                }
            }
            Some(base) if resized => {
                if (base.0 - new_size.0).abs() > 0.5 || (base.1 - new_size.1).abs() > 0.5 {
                    self.reflow_windows(base, new_size);
                } else {
                    // Back at the base size: the base rects are the live
                    // rects again, exactly as laid out.
                    for win in &mut self.windows {
                        win.rect = win.base_rect;
                        if win.maximised {
                            win.restore = Some(win.base_rect);
                        }
                    }
                }
                cx.notify();
            }
            Some(_) => {}
        }
    }

    /// Derives live rects from the base layout at a new painted size.
    /// The base itself is untouched, so growing back restores it exactly.
    fn reflow_windows(&mut self, from: (f32, f32), to: (f32, f32)) {
        if self.windows.is_empty() {
            return;
        }
        let rects: Vec<Rect> = self.windows.iter().map(|win| win.base_rect).collect();
        let mins: Vec<(f32, f32)> = self.windows.iter().map(|win| win.kind.min_size()).collect();
        let flowed = geometry::reflow(
            &rects,
            Rect::new(0., 0., from.0, from.1),
            Rect::new(0., 0., to.0, to.1),
            &mins,
        );
        for (win, rect) in self.windows.iter_mut().zip(flowed) {
            win.rect = rect;
            if win.maximised {
                win.restore = Some(rect);
            }
        }
    }

    /// Makes the live layout the base: after a user edit, the base is
    /// what is on screen, laid out for the current painted size.
    fn rebase(&mut self) {
        for win in &mut self.windows {
            win.base_rect = if win.maximised {
                win.restore.unwrap_or(win.rect)
            } else {
                win.rect
            };
        }
        self.base_size = Some(self.size);
    }

    /// The rect a window renders at: the whole canvas when maximised, else
    /// its saved rect clamped so the title stays reachable. The saved rect
    /// is not rewritten.
    fn display_rect(&self, win: &CanvasWindow) -> Rect {
        if win.maximised {
            return self.canvas_rect().inset(GUTTER);
        }
        geometry::clamp_to_canvas(win.rect, self.canvas_rect(), TITLE_HEIGHT)
    }

    // ---- mouse -----------------------------------------------------------

    fn local(&self, position: GpuiPoint<Pixels>) -> Point {
        let bounds = self.bounds.get();
        Point {
            x: position.x.as_f32() - bounds.x,
            y: position.y.as_f32() - bounds.y,
        }
    }

    fn others(&self, except: u64) -> Vec<Rect> {
        self.windows
            .iter()
            .filter(|win| win.id != except && !win.minimised)
            .map(|win| self.display_rect(win))
            .collect()
    }

    /// A press anywhere in a window: bring it to the front and focus it.
    fn window_mouse_down(&mut self, id: u64, cx: &mut Context<Self>) {
        let was_front = self.windows.last().is_some_and(|win| win.id == id);
        self.bring_to_front(id);
        if self.focused != Some(id) || !was_front {
            self.focused = Some(id);
            cx.notify();
        }
    }

    /// A press on the title strip: start a move, or toggle maximise on the
    /// second click.
    pub(crate) fn title_mouse_down(
        &mut self,
        id: u64,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.window_mouse_down(id, cx);
        if let Some(win) = self.window(id) {
            let handle = win.view.focus_handle(cx);
            window.focus(&handle, cx);
        }
        if event.click_count >= 2 {
            self.toggle_maximise(id, cx);
            return;
        }
        let Some(win) = self.window(id) else {
            return;
        };
        if win.maximised {
            return;
        }
        self.drag = Some(Drag {
            id,
            kind: DragKind::Move,
            start: self.local(event.position),
            start_rect: win.rect,
        });
    }

    /// A press on an edge grip: start a resize.
    fn grip_mouse_down(
        &mut self,
        id: u64,
        edge: Edge,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        self.window_mouse_down(id, cx);
        let Some(win) = self.window(id) else {
            return;
        };
        if win.maximised {
            return;
        }
        self.drag = Some(Drag {
            id,
            kind: DragKind::Resize(edge),
            start: self.local(event.position),
            start_rect: win.rect,
        });
    }

    fn drag_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = &self.drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let (id, kind, start, start_rect) = (drag.id, drag.kind, drag.start, drag.start_rect);
        let at = self.local(event.position);
        let canvas = self.canvas_rect();
        let others = self.others(id);
        let Some(win) = self.window_mut(id) else {
            self.drag = None;
            return;
        };
        let min = win.kind.min_size();
        let snapped = match kind {
            DragKind::Move => {
                let moved = Rect::new(
                    start_rect.x + at.x - start.x,
                    start_rect.y + at.y - start.y,
                    start_rect.w,
                    start_rect.h,
                );
                geometry::snap_move(moved, &others, canvas, SNAP_THRESHOLD)
            }
            DragKind::Resize(edge) => {
                let resized =
                    geometry::resize_rect(start_rect, edge, at.x - start.x, at.y - start.y, min);
                let snapped = geometry::snap_resize(resized, edge, &others, canvas, SNAP_THRESHOLD);
                geometry::Snapped {
                    rect: geometry::enforce_min(snapped.rect, min),
                    guides: snapped.guides,
                }
            }
        };
        win.rect = geometry::clamp_to_canvas(snapped.rect, canvas, TITLE_HEIGHT);
        self.guides = snapped.guides;
        cx.notify();
    }

    fn drag_end(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            self.guides.clear();
            self.rebase();
            self.changed(cx);
        }
    }

    // ---- rendering ---------------------------------------------------------

    fn empty_state(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .id("canvas-empty")
            .test_support()
            .absolute()
            .inset_0()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                theme::serif(self.screen_name.clone())
                    .text_size(crate::ui::rem(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground),
            )
            .child(
                div()
                    .text_size(crate::ui::rem(13.))
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "Open a window with {}, or start from a template",
                        crate::theme::command_key()
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("canvas-open-book-trader")
                            .label("Book Trader")
                            .small()
                            .outline()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open(PaneKind::BookTrader, window, cx);
                            })),
                    )
                    .child(
                        Button::new("canvas-trading-template")
                            .label("Trading template")
                            .small()
                            .outline()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.place_fractions(
                                    &crate::layouts::Preset::Trading.fractions(),
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("canvas-market-making-template")
                            .label("Market Making template")
                            .small()
                            .outline()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.place_fractions(
                                    &crate::layouts::Preset::MarketMaking.fractions(),
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
    }

    fn grip(
        id: u64,
        edge: Edge,
        cx: &mut Context<Self>,
    ) -> gpui_kit::base::ObservedElement<gpui_kit::Stateful<Div>> {
        let name = match edge {
            Edge::Left => "left",
            Edge::Right => "right",
            Edge::Top => "top",
            Edge::Bottom => "bottom",
            Edge::TopLeft => "top-left",
            Edge::TopRight => "top-right",
            Edge::BottomLeft => "bottom-left",
            Edge::BottomRight => "bottom-right",
        };
        let base = div()
            .id(SharedString::from(format!("win-grip-{id}-{name}")))
            .test_support()
            .absolute();
        let placed = match edge {
            Edge::Left => base
                .left_0()
                .top(px(8.))
                .bottom(px(8.))
                .w(px(6.))
                .cursor_col_resize(),
            Edge::Right => base
                .right_0()
                .top(px(8.))
                .bottom(px(8.))
                .w(px(6.))
                .cursor_col_resize(),
            Edge::Top => base
                .top_0()
                .left(px(8.))
                .right(px(8.))
                .h(px(6.))
                .cursor_row_resize(),
            Edge::Bottom => base
                .bottom_0()
                .left(px(8.))
                .right(px(8.))
                .h(px(6.))
                .cursor_row_resize(),
            Edge::TopLeft => base.left_0().top_0().size(px(12.)).cursor_nwse_resize(),
            Edge::BottomRight => base.right_0().bottom_0().size(px(12.)).cursor_nwse_resize(),
            Edge::TopRight => base.right_0().top_0().size(px(12.)).cursor_nesw_resize(),
            Edge::BottomLeft => base.left_0().bottom_0().size(px(12.)).cursor_nesw_resize(),
        };
        placed.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event, _, cx| this.grip_mouse_down(id, edge, event, cx)),
        )
    }

    fn window_element(&self, win: &CanvasWindow, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let rect = self.display_rect(win);
        let focused = self.focused == Some(win.id);
        let id = win.id;
        let instrument = if win.kind.follows_instrument() {
            self.desk
                .read(cx)
                .instrument()
                .map(|instrument| instrument.symbol)
        } else {
            None
        };
        let mut element = v_flex()
            .id(SharedString::from(format!("window-{id}")))
            .test_support()
            .group("canvas-window")
            // A window blocks the mouse for everything behind it, grips of
            // background windows included.
            .occlude()
            .absolute()
            .left(px(rect.x))
            .top(px(rect.y))
            .w(px(rect.w))
            .h(px(rect.h))
            .bg(theme.table)
            .border_1()
            .border_color(if focused {
                theme.ring.alpha(0.6)
            } else {
                theme.border
            })
            .rounded(px(6.))
            .overflow_hidden()
            .when(focused, gpui_kit::Styled::shadow_md)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| this.window_mouse_down(id, cx)),
            )
            // Drag continues while the pointer is over this window; the
            // canvas root's handlers cover the gaps between windows
            // (occlusion keeps each from seeing through the other).
            .on_mouse_move(cx.listener(|this, event, _, cx| this.drag_move(event, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.drag_end(cx)),
            )
            .child(frame::window_frame(
                id,
                win.kind,
                instrument,
                focused,
                win.maximised,
                cx,
            ))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(win.view.view()),
            );
        if !win.maximised {
            for edge in [
                Edge::Left,
                Edge::Right,
                Edge::Top,
                Edge::Bottom,
                Edge::TopLeft,
                Edge::TopRight,
                Edge::BottomLeft,
                Edge::BottomRight,
            ] {
                element = element.child(Self::grip(id, edge, cx));
            }
        }
        element.into_any_element()
    }
}

impl Render for Canvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.settle_size(window, cx);
        let (background, guide_color) = {
            let theme = cx.theme();
            (theme.background, theme.drag_border)
        };
        let bounds = self.bounds.clone();
        let empty = self.windows.is_empty() && !self.pending_template;
        let mut root = div()
            .id("canvas")
            .test_support()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(background)
            .on_mouse_move(cx.listener(|this, event, _, cx| this.drag_move(event, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.drag_end(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.drag_end(cx)),
            )
            .child(
                paint_canvas(
                    move |painted_bounds, _, _| {
                        bounds.set(Rect::new(
                            painted_bounds.origin.x.as_f32(),
                            painted_bounds.origin.y.as_f32(),
                            painted_bounds.size.width.as_f32(),
                            painted_bounds.size.height.as_f32(),
                        ));
                    },
                    |_, (), _, _| {},
                )
                .absolute()
                .inset_0(),
            );
        let _ = window;
        for index in 0..self.windows.len() {
            if self.windows[index].minimised {
                continue;
            }
            let element = self.window_element(&self.windows[index], cx);
            root = root.child(element);
        }
        for guide in &self.guides {
            root = root.child(match guide {
                Guide::Vertical(x) => div()
                    .absolute()
                    .left(px(*x))
                    .top_0()
                    .bottom_0()
                    .w(px(1.))
                    .bg(guide_color)
                    .into_any_element(),
                Guide::Horizontal(y) => div()
                    .absolute()
                    .top(px(*y))
                    .left_0()
                    .right_0()
                    .h(px(1.))
                    .bg(guide_color)
                    .into_any_element(),
            });
        }
        if empty {
            root = root.child(self.empty_state(cx));
        }
        root
    }
}
