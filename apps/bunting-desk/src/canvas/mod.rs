//! The canvas of floating windows (ADR 0051 decisions 2 and 7): each pane
//! lives in a window that can be moved, resized, minimised, maximised and
//! closed, drawn with the one [`WindowFrame`]. Pure geometry lives in
//! [`geometry`]; this module holds the GPUI entity and rendering.

#[allow(
    clippy::float_arithmetic,
    reason = "window geometry is f32 logical pixels throughout"
)]
pub mod geometry;
