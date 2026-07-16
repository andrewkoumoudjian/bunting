// Adapted from cli-candlestick-chart 0.24.0 at Longbridge commit
// 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright (c) 2021 Julien-R44. Licensed under MIT.

#![allow(
    dead_code,
    clippy::all,
    clippy::pedantic,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_arithmetic,
    clippy::unwrap_used
)]

mod candle_set;
mod chart;
mod chart_data;
mod chart_renderer;
mod info_bar;
mod volume_pane;
mod y_axis;

pub use chart::{Candle, Chart};
