use ratatui::prelude::*;
use ratatui::style::Styled;
use ratatui::symbols::bar::NINE_LEVELS;
use ratatui::widgets::{Block, RenderDirection, WidgetRef};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Sparkline<'a> {
    /// A block to wrap the widget in
    block: Option<Block<'a>>,
    /// Widget style
    style: Style,
    /// A slice of the data to display
    data: &'a [u64],
    /// The maximum value to take to compute the maximum bar height (if nothing is specified, the
    /// widget uses the max of the dataset)
    max: Option<u64>,
    // The direction to render the sparkine, either from left to right, or from right to left
    direction: RenderDirection,
    inverted: bool,
}

impl<'a> Default for Sparkline<'a> {
    fn default() -> Self {
        Self {
            block: None,
            style: Style::default(),
            data: &[],
            max: None,
            direction: RenderDirection::LeftToRight,
            inverted: false,
        }
    }
}

impl<'a> Sparkline<'a> {
    /// Wraps the sparkline with the given `block`.
    #[must_use = "method moves the value of self and returns the modified value"]
    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }

    /// Sets the style of the entire widget.
    ///
    /// `style` accepts any type that is convertible to [`Style`] (e.g. [`Style`], [`Color`], or
    /// your own type that implements [`Into<Style>`]).
    ///
    /// The foreground corresponds to the bars while the background is everything else.
    #[must_use = "method moves the value of self and returns the modified value"]
    pub fn style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = style.into();
        self
    }

    /// Sets the dataset for the sparkline.
    ///
    /// # Example
    ///
    /// ```
    /// # use ratatui::{prelude::*, widgets::*};
    /// # fn ui(frame: &mut Frame) {
    /// # let area = Rect::default();
    /// let sparkline = Sparkline::default().data(&[1, 2, 3]);
    /// frame.render_widget(sparkline, area);
    /// # }
    /// ```
    #[must_use = "method moves the value of self and returns the modified value"]
    pub const fn data(mut self, data: &'a [u64]) -> Self {
        self.data = data;
        self
    }

    /// Sets the maximum value of bars.
    ///
    /// Every bar will be scaled accordingly. If no max is given, this will be the max in the
    /// dataset.
    #[must_use = "method moves the value of self and returns the modified value"]
    #[allow(unused)]
    pub const fn max(mut self, max: Option<u64>) -> Self {
        self.max = max;
        self
    }

    pub const fn inverted(mut self, inverted: bool) -> Self {
        self.inverted = inverted;
        self
    }

    /// Sets the direction of the sparkline.
    ///
    /// [`RenderDirection::LeftToRight`] by default.
    #[must_use = "method moves the value of self and returns the modified value"]
    pub const fn direction(mut self, direction: RenderDirection) -> Self {
        self.direction = direction;
        self
    }
}

impl<'a> Styled for Sparkline<'a> {
    type Item = Self;

    fn style(&self) -> Style {
        if self.inverted {
            self.style.add_modifier(Modifier::REVERSED)
        } else {
            self.style
        }
    }

    fn set_style<S: Into<Style>>(self, style: S) -> Self::Item {
        self.style(style)
    }
}

impl Widget for Sparkline<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_ref(area, buf);
    }
}

impl WidgetRef for Sparkline<'_> {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        self.block.render_ref(area, buf);
        let inner = self.block.inner_if_some(area);
        self.render_sparkline(inner, buf);
    }
}

impl Sparkline<'_> {
    fn resolve_symbol(&self, value: u64) -> &str {
        let bar_set = NINE_LEVELS;
        if self.inverted {
            match value {
                0 => bar_set.full,
                7 => bar_set.one_eighth,
                6 => bar_set.one_quarter,
                5 => bar_set.three_eighths,
                4 => bar_set.half,
                3 => bar_set.five_eighths,
                2 => bar_set.three_quarters,
                1 => bar_set.seven_eighths,
                _ => bar_set.empty,
            }
        } else {
            match value {
                0 => bar_set.empty,
                1 => bar_set.one_eighth,
                2 => bar_set.one_quarter,
                3 => bar_set.three_eighths,
                4 => bar_set.half,
                5 => bar_set.five_eighths,
                6 => bar_set.three_quarters,
                7 => bar_set.seven_eighths,
                _ => bar_set.full,
            }
        }
    }

    fn render_sparkline(&self, spark_area: Rect, buf: &mut Buffer) {
        if spark_area.is_empty() {
            return;
        }

        let max = match self.max {
            Some(v) => v,
            None => *self.data.iter().max().unwrap_or(&1),
        };
        let max_index = std::cmp::min(spark_area.width as usize, self.data.len());
        let mut data = self
            .data
            .iter()
            .take(max_index)
            .map(|e| {
                if max == 0 {
                    0
                } else {
                    e * u64::from(spark_area.height) * 8 / max
                }
            })
            .collect::<Vec<u64>>();

        let row_order: Vec<_> = if self.inverted {
            (0..spark_area.height).collect()
        } else {
            (0..spark_area.height).rev().collect()
        };

        for j in row_order {
            for (i, d) in data.iter_mut().enumerate() {
                let symbol = self.resolve_symbol(*d);
                let x = match self.direction {
                    RenderDirection::LeftToRight => spark_area.left() + i as u16,
                    RenderDirection::RightToLeft => spark_area.right() - i as u16 - 1,
                };
                if let Some(cell) = buf.cell_mut(Position::new(x, spark_area.top() + j)) {
                    cell.set_symbol(symbol).set_style(self.style());
                }

                if *d > 8 {
                    *d -= 8;
                } else {
                    *d = 0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Cell;

    #[test]
    fn it_draws() {
        let widget = Sparkline::default().data(&[0, 1, 2, 3, 4, 5, 6, 7, 8]);
        let buffer = render(&widget, 12, 1);
        assert_eq!(buffer, Buffer::with_lines(vec![" ▁▂▃▄▅▆▇█xxx"]));

        let buffer = render(&widget, 12, 2);
        assert_eq!(
            buffer,
            Buffer::with_lines(vec![
                "     ▂▄▆█xxx", //
                " ▂▄▆█████xxx", //
            ])
        );
    }

    fn reversed(mut b: Buffer) -> Buffer {
        let area = b.area().clone();
        b.set_style(area, Style::default().add_modifier(Modifier::REVERSED));
        b
    }

    #[test]
    fn it_draws_inverted() {
        let widget = Sparkline::default()
            .data(&[0, 1, 2, 3, 4, 5, 6, 7, 8])
            .inverted(true);
        let buffer = render(&widget, 9, 1);
        assert_eq!(buffer, reversed(Buffer::with_lines(vec!["█▇▆▅▄▃▂▁ "])));

        let buffer = render(&widget, 9, 2);
        assert_eq!(
            buffer,
            reversed(Buffer::with_lines(vec![
                "█▆▄▂     ", //
                "█████▆▄▂ ", //
            ]))
        );
    }

    // Helper function to render a sparkline to a buffer with a given width
    // filled with x symbols to make it easier to assert on the result
    fn render(widget: &Sparkline, width: u16, height: u16) -> Buffer {
        let area = Rect::new(0, 0, width, height);
        let mut cell = Cell::default();
        cell.set_symbol("x");
        let mut buffer = Buffer::filled(area, cell);
        widget.render(area, &mut buffer);
        buffer
    }
}
