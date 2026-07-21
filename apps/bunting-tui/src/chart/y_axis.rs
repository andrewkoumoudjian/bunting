// Adapted from cli-candlestick-chart 0.24.0 at Longbridge commit
// 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3. MIT licensed.

use std::{cell::RefCell, rc::Rc};

use super::chart_data::ChartData;

pub struct YAxis {
    pub chart_data: Rc<RefCell<ChartData>>,
}

impl YAxis {
    pub const CHAR_PRECISION: i64 = 6;
    pub const DEC_PRECISION: i64 = 2;
    pub const MARGIN_RIGHT: i64 = 4;

    pub const WIDTH: i64 = YAxis::CHAR_PRECISION
        + YAxis::MARGIN_RIGHT
        + 1
        + YAxis::DEC_PRECISION
        + YAxis::MARGIN_RIGHT;

    pub fn new(chart_data: Rc<RefCell<ChartData>>) -> YAxis {
        YAxis { chart_data }
    }

    pub fn price_to_height(&self, price: f64) -> f64 {
        let chart_data = self.chart_data.borrow();
        let min_value = chart_data.visible_candle_set.min_price;
        let max_value = chart_data.visible_candle_set.max_price;
        let (bottom, top) = drawable_bounds(chart_data.height);
        let price_range = max_value - min_value;

        if price_range.abs() <= f64::EPSILON {
            return bottom.midpoint(top);
        }
        bottom + ((price - min_value) / price_range).clamp(0.0, 1.0) * (top - bottom)
    }

    pub fn render_line(&self, y: u16) -> String {
        let top = self.chart_data.borrow().height.saturating_sub(1).max(1) as u16;
        if y == 1 || y == top || y.is_multiple_of(4) {
            self.render_tick(y)
        } else {
            self.render_empty()
        }
    }

    fn render_tick(&self, y: u16) -> String {
        let chart_data = self.chart_data.borrow();
        let min_value = chart_data.visible_candle_set.min_price;
        let max_value = chart_data.visible_candle_set.max_price;
        let (bottom, top) = drawable_bounds(chart_data.height);
        let span = (top - bottom).max(1.0);
        let normalized = ((f64::from(y) - bottom) / span).clamp(0.0, 1.0);
        let price = min_value + normalized * (max_value - min_value);
        let cell_min_length = (YAxis::CHAR_PRECISION + YAxis::DEC_PRECISION + 1) as usize;

        format!(
            "{0:<cell_min_length$.2} │┈{margin}",
            price,
            cell_min_length = cell_min_length,
            margin = " ".repeat(YAxis::MARGIN_RIGHT as usize)
        )
    }

    pub fn render_empty(&self) -> String {
        let cell = " ".repeat((YAxis::CHAR_PRECISION + YAxis::DEC_PRECISION + 2) as usize);
        let margin = " ".repeat((YAxis::MARGIN_RIGHT + 1).try_into().unwrap());

        format!("{}│{}", cell, margin)
    }
}

fn drawable_bounds(height: i64) -> (f64, f64) {
    (1.0, height.saturating_sub(1).max(1) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::{Candle, chart_data::ChartData};

    fn axis(height: i64, low: f64, high: f64) -> YAxis {
        let data = Rc::new(RefCell::new(ChartData::new(
            vec![Candle::new(100.0, high, low, 101.0, None, None)],
            (40, u16::try_from(height).unwrap()),
        )));
        data.borrow_mut().height = height;
        YAxis::new(data)
    }

    #[test]
    fn extrema_scale_to_dynamic_drawable_rows() {
        let short = axis(8, 97.0, 127.0);
        assert_eq!(short.price_to_height(97.0), 1.0);
        assert_eq!(short.price_to_height(127.0), 7.0);
        assert!(short.render_line(1).starts_with("97.00"));
        assert!(short.render_line(7).starts_with("127.00"));

        let tall = axis(20, 97.0, 127.0);
        assert_eq!(tall.price_to_height(97.0), 1.0);
        assert_eq!(tall.price_to_height(127.0), 19.0);
    }

    #[test]
    fn flat_market_uses_the_dynamic_vertical_midpoint() {
        let axis = axis(10, 100.0, 100.0);
        assert_eq!(axis.price_to_height(100.0), 5.0);
    }
}
