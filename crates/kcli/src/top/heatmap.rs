use crate::top::state::fvalue_with_unit;
use crate::top::Histogram;
use colorgrad::Gradient;
use ratatui::prelude::*;
use ratatui::style::Styled;
use ratatui::widgets::{Block, WidgetRef};

#[derive(Debug, Clone, PartialEq)]
pub struct HeatMap<'a> {
    /// A block to wrap the widget in
    block: Option<Block<'a>>,
    /// Widget style
    style: Style,
    histogram: &'a Histogram,
    caption: Option<&'a str>,
}

impl<'a> Styled for HeatMap<'a> {
    type Item = Self;

    fn style(&self) -> Style {
        self.style
    }

    fn set_style<S: Into<Style>>(mut self, style: S) -> Self::Item {
        self.style = style.into();
        self
    }
}

impl Widget for HeatMap<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_ref(area, buf);
    }
}

impl WidgetRef for HeatMap<'_> {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        self.block.render_ref(area, buf);
        let inner = self.block.inner_if_some(area);
        self.render_heatmap(inner, buf);
    }
}

impl<'a> HeatMap<'a> {
    pub fn new(histogram: &'a Histogram, caption: &'a str) -> Self {
        Self {
            histogram,
            style: Style::default(),
            block: None,
            caption: Some(caption),
        }
    }

    #[must_use = "method moves the value of self and returns the modified value"]
    pub fn block(mut self, block: Block<'a>) -> Self {
        let block = match self.caption.take() {
            Some(title) => block.title(title),
            None => block,
        };
        self.block = Some(block);
        self
    }

    /// Returns the number of rows that would render inside the block
    pub fn height(&self) -> u16 {
        let n = self.histogram.buckets.len() + 1 /* gradient/legend */
            + if self.caption.is_some() { 1 } else { 0 };

        n as u16
    }

    fn render_heatmap(&self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }

        let mut first_row = 0;

        if let Some(caption) = &self.caption {
            buf.set_string(0, area.top(), caption, Style::default());
            first_row += 1;
        }

        let labels: Vec<String> = self
            .histogram
            .buckets
            .iter()
            .map(|le| {
                if *le == f64::INFINITY {
                    "Inf".to_string()
                } else {
                    fvalue_with_unit(*le, &self.histogram.unit)
                }
            })
            .collect();

        let label_width = labels.iter().map(|l| l.len()).max().unwrap_or(0) + 1;

        for (y, label) in labels.iter().rev().enumerate() {
            buf.set_string(
                area.left(),
                area.top() + first_row + y as u16,
                label,
                Style::default(),
            );
        }

        let max_width = (area.width as usize - label_width).min(self.histogram.data.len());

        let mut cols = vec![];
        let mut max_delta = 1;
        let mut accum: Option<Vec<u64>> = None;
        for col in self
            .histogram
            .data
            .iter()
            .skip(self.histogram.data.len() - max_width)
            .take(max_width)
        {
            let row_deltas: Vec<u64> = match &accum {
                None => col.iter().map(|_| 0).collect(),
                Some(a) => a
                    .iter()
                    .zip(col.iter())
                    .map(|(a, b)| b.saturating_sub(*a))
                    .collect(),
            };
            accum.replace(col.to_vec());

            let mut prior = 0;
            let data: Vec<_> = row_deltas
                .iter()
                .map(|&v| {
                    let delta = v - prior;
                    prior = v;
                    max_delta = max_delta.max(delta);
                    delta
                })
                .collect();
            cols.push(data);
        }

        let max_delta = max_delta as f32;

        let right_alignment = area.width as usize - cols.len();

        for (x, data) in cols.into_iter().enumerate() {
            for (y, value) in data.into_iter().rev().enumerate() {
                if value == 0 {
                    continue;
                }
                let color = color(value as f32 / max_delta);
                buf[Position::new(
                    (right_alignment + x) as u16,
                    area.top() + first_row + y as u16,
                )]
                .set_char(' ')
                .set_bg(color);
            }
        }

        // Draw a scale/legend underneath to help understand the hot spots
        let scale_y = area.top() + first_row + labels.len() as u16;

        let max_delta_str = max_delta.to_string();
        let avail = area.width as usize - (label_width + 1 + max_delta_str.len() + 2);
        let step = max_delta / avail as f32;
        for x in 0..avail {
            let color = color(x as f32 * step / max_delta);
            buf[Position::new(area.left() + (label_width + x) as u16, scale_y)]
                .set_char(' ')
                .set_bg(color);
        }
        buf.set_string(
            area.left() + label_width as u16,
            scale_y,
            format!("1"),
            Style::default(),
        );
        buf.set_string(
            area.left() + (label_width + avail) as u16,
            scale_y,
            max_delta_str,
            Style::default(),
        );
    }
}

fn color(percentage: f32) -> Color {
    let gradient = colorgrad::preset::turbo();
    let [r, g, b, _a] = gradient.at(percentage).to_rgba8();
    Color::from_u32(((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
}
