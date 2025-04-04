use crate::queue_summary::get_metrics;
use crate::top::heatmap::HeatMap;
use crate::top::histogram::*;
use crate::top::sparkline::Sparkline;
use crate::top::timeseries::*;
use crate::top::{Action, TopCommand, WhichTab};
use human_bytes::human_bytes;
use kumo_prometheus::parser::Metric;
use num_format::{Locale, ToFormattedString};
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Clear, Paragraph, RenderDirection, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Tabs, Wrap,
};
use reqwest::Url;
use std::collections::HashMap;

struct Entry<'a> {
    label: &'a str,
    data: &'a [u64],
    color: Color,
    inverted: bool,
    unit: &'a str,
    base_height: u16,
}

impl<'a> Entry<'a> {
    fn new(
        label: &'a str,
        data: &'a [u64],
        color: Color,
        inverted: bool,
        unit: &'a str,
        base_height: u16,
    ) -> Self {
        Self {
            label,
            data,
            color,
            inverted,
            unit,
            base_height,
        }
    }

    fn current_value(&self) -> String {
        if self.base_height == 1 {
            String::new()
        } else {
            self.current_value_impl()
        }
    }

    fn current_value_impl(&self) -> String {
        self.data
            .get(0)
            .map(|v| value_with_unit(*v, self.unit))
            .unwrap_or_else(String::new)
    }

    fn label(&self, col_width: Option<u16>) -> String {
        if self.base_height == 1 {
            let value = self.current_value_impl();

            let spacing = if let Some(col_width) = col_width {
                " ".repeat(col_width as usize - (value.len() + self.label.len()))
            } else {
                "  ".to_string()
            };

            format!("{}{spacing}{value}", self.label)
        } else {
            self.label.to_string()
        }
    }

    fn min_width(&self) -> u16 {
        (self.current_value().len() + 2).max(self.label(None).len() + 2) as u16
    }
}

#[derive(Default)]
pub struct State {
    time_series: HashMap<String, TimeSeries>,
    histograms: HashMap<String, Histogram>,
    factories: Vec<Box<dyn SeriesFactory + 'static>>,
    error: String,
    vert_scroll: ScrollbarState,
    vert_scroll_position: usize,
    zoom: u8,
    active_tab: WhichTab,
}

impl State {
    pub fn add_factory(&mut self, f: impl SeriesFactory + 'static) {
        self.factories.push(Box::new(f));
    }

    fn accumulate_series(&mut self, metric: &Metric) {
        let mut new_series = vec![];
        for factory in &self.factories {
            if let Some(name) = factory.matches(metric) {
                if !self.time_series.contains_key(&name) {
                    let series = factory.factory(&name, metric);
                    new_series.push((name, series));
                }
            }
        }
        for (name, series) in new_series {
            self.add_series(name, series);
        }

        for series in self.time_series.values_mut() {
            series.accumulate(metric);
        }

        for histo in self.histograms.values_mut() {
            histo.accumulate(metric);
        }
    }

    fn commit_series(&mut self) {
        for series in self.time_series.values_mut() {
            series.commit();
        }
    }

    fn get_series(&self, name: &str) -> Option<&TimeSeries> {
        self.time_series.get(name)
    }

    pub fn add_series<S: Into<String>>(&mut self, name: S, series: TimeSeries) {
        self.time_series.insert(name.into(), series);
    }

    pub fn add_histogram<S: Into<String>>(&mut self, name: S, histo: Histogram) {
        self.histograms.insert(name.into(), histo);
    }

    async fn update_metrics(&mut self, endpoint: &Url) -> anyhow::Result<()> {
        match get_metrics::<Vec<()>, _>(endpoint, |m| {
            self.accumulate_series(&m);
            None
        })
        .await
        {
            Ok(_) => {
                self.error.clear();
                self.commit_series();
            }
            Err(err) => {
                self.error = format!("{err:#}");
                self.commit_series();
            }
        }
        Ok(())
    }

    pub async fn update(&mut self, action: Action, endpoint: &Url) -> anyhow::Result<()> {
        match action {
            Action::Quit => anyhow::bail!("quit!"),
            Action::NextTab => {
                self.active_tab.next();
            }
            Action::UpdateData => self.update_metrics(endpoint).await?,
            Action::ScrollTop => {
                self.vert_scroll_position = 0;
            }
            Action::ScrollBottom => {
                self.vert_scroll_position = usize::MAX;
            }
            Action::ScrollUp => {
                self.vert_scroll_position = self.vert_scroll_position.saturating_sub(1);
            }
            Action::ScrollDown => {
                self.vert_scroll_position = self.vert_scroll_position.saturating_add(1);
            }
            Action::PageUp => {
                self.vert_scroll_position = self.vert_scroll_position.saturating_sub(10);
            }
            Action::PageDown => {
                self.vert_scroll_position = self.vert_scroll_position.saturating_add(10);
            }
            Action::ZoomIn => {
                self.zoom = self.zoom.saturating_add(1);
            }
            Action::ZoomOut => {
                self.zoom = self.zoom.saturating_sub(1);
            }
            Action::Redraw => {}
        }
        Ok(())
    }

    fn draw_series_ui(&mut self, f: &mut Frame) {
        let mut sparklines = vec![
            Entry::new(
                "Delivered",
                &self
                    .get_series("total_messages_delivered_rate")
                    .unwrap()
                    .data,
                Color::Green,
                false,
                "/s",
                2,
            ),
            Entry::new(
                "Received",
                &self
                    .get_series("total_messages_received_rate")
                    .unwrap()
                    .data,
                Color::LightGreen,
                true,
                "/s",
                2,
            ),
            Entry::new(
                "Transfail",
                &self
                    .get_series("total_messages_transfail_rate")
                    .unwrap()
                    .data,
                Color::Red,
                false,
                "/s",
                2,
            ),
            Entry::new(
                "Permfail",
                &self.get_series("total_messages_fail_rate").unwrap().data,
                Color::LightRed,
                false,
                "/s",
                2,
            ),
            Entry::new(
                "Scheduled",
                &self.get_series("scheduled_count_total").unwrap().data,
                Color::Green,
                false,
                "",
                2,
            ),
            Entry::new(
                "Ready",
                &self.get_series("ready_count").unwrap().data,
                Color::LightGreen,
                false,
                "",
                2,
            ),
            Entry::new(
                "Messages",
                &self.get_series("message_count").unwrap().data,
                Color::Green,
                false,
                "",
                2,
            ),
            Entry::new(
                "Resident",
                &self.get_series("message_data_resident_count").unwrap().data,
                Color::LightGreen,
                true,
                "",
                1,
            ),
            Entry::new(
                "Memory",
                &self.get_series("memory_usage").unwrap().data,
                Color::Green,
                false,
                "b",
                2,
            ),
            Entry::new(
                "Conn Out",
                &self.get_series("smtp_conns").unwrap().data,
                Color::LightGreen,
                false,
                "",
                2,
            ),
            Entry::new(
                "Conn In",
                &self.get_series("listener_conns").unwrap().data,
                Color::Green,
                true,
                "",
                2,
            ),
        ];

        let pool_colors = [Color::LightGreen, Color::Green];
        let mut dynamic_series = self
            .time_series
            .iter()
            .filter(|(_name, series)| series.chart.is_some())
            .collect::<Vec<_>>();
        dynamic_series
            .sort_by_key(|(_series_name, series)| series.chart.as_ref().unwrap().name.clone());

        for (series_name, series) in dynamic_series {
            if let Some(chart) = &series.chart {
                let next_idx = sparklines.len();
                sparklines.push(Entry::new(
                    &chart.name,
                    &self.get_series(series_name).unwrap().data,
                    pool_colors[next_idx % pool_colors.len()],
                    chart.inverted,
                    &chart.unit,
                    1,
                ));
            }
        }

        for entry in sparklines.iter_mut() {
            entry.base_height += self.zoom as u16;
        }

        let label_col_width = sparklines
            .iter()
            .map(|entry| entry.min_width())
            .max()
            .unwrap_or(12);

        let content_length = sparklines.len();
        let vert_scroll_position = self
            .vert_scroll_position
            .min(content_length.saturating_sub(1));

        let mut y = 1;
        for entry in sparklines.into_iter().skip(vert_scroll_position) {
            if y >= f.area().height || y + entry.base_height >= f.area().height {
                break;
            }

            let spark_chunk = Rect {
                x: 0,
                y,
                width: f.area().width - label_col_width - 1,
                height: entry.base_height,
            };
            let summary = Rect {
                x: f.area().width - label_col_width - 1,
                y,
                width: label_col_width,
                height: entry.base_height,
            };

            y += entry.base_height;

            let text_style = Style::default()
                .fg(entry.color)
                .add_modifier(Modifier::REVERSED);

            let sparkline = Sparkline::default()
                .block(Block::new().borders(Borders::RIGHT))
                .data(entry.data)
                .direction(RenderDirection::RightToLeft)
                .inverted(entry.inverted)
                .max(if entry.unit == "%" { Some(100) } else { None })
                .style(Style::default().fg(entry.color));
            f.render_widget(sparkline, spark_chunk);

            let label = Paragraph::new(entry.current_value())
                .right_aligned()
                .style(text_style.clone())
                .block(
                    Block::new()
                        .title(entry.label(Some(label_col_width)))
                        .title_style(text_style.clone())
                        .title_alignment(if entry.base_height == 1 {
                            Alignment::Right
                        } else {
                            Alignment::Left
                        }),
                );
            f.render_widget(label, summary);
        }

        self.vert_scroll_position = vert_scroll_position;
        self.vert_scroll = self
            .vert_scroll
            .content_length(content_length)
            .position(self.vert_scroll_position);

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));

        f.render_stateful_widget(
            scrollbar,
            f.area().inner(Margin {
                // using an inner vertical margin of 1 unit makes the scrollbar inside the block
                vertical: 1,
                horizontal: 0,
            }),
            &mut self.vert_scroll,
        );
    }

    fn draw_heatmap_ui(&mut self, f: &mut Frame) {
        let area = f.area().inner(Margin {
            vertical: 2,
            horizontal: 2,
        });

        let content_length = self.histograms.len();
        let vert_scroll_position = self
            .vert_scroll_position
            .min(content_length.saturating_sub(1));

        let mut y = area.top();
        for (caption, histo) in self.histograms.iter().skip(vert_scroll_position) {
            let heatmap = HeatMap::new(histo, caption).block(Block::bordered());
            let height = heatmap.height() + 2 /* borders */;

            if y > area.bottom() || y + height > area.bottom() {
                break;
            }

            f.render_widget(
                heatmap,
                Rect {
                    x: 0,
                    y,
                    width: area.width,
                    height,
                },
            );

            y += height;
        }

        self.vert_scroll_position = vert_scroll_position;
        self.vert_scroll = self
            .vert_scroll
            .content_length(content_length)
            .position(self.vert_scroll_position);

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));

        f.render_stateful_widget(
            scrollbar,
            f.area().inner(Margin {
                // using an inner vertical margin of 1 unit makes the scrollbar inside the block
                vertical: 1,
                horizontal: 0,
            }),
            &mut self.vert_scroll,
        );
    }

    fn draw_help_ui(&mut self, f: &mut Frame) {
        let paragraph = Paragraph::new(vec![
            Line::from("Use Tab to switch between tabs"),
            Line::from("Escape or 'q' to quit"),
            Line::from("▲ ▼ to scroll up or down"),
            Line::from("Page Up or Page Down to scroll up or down faster"),
            Line::from("Home or End to scroll to the top or bottom"),
            Line::from("'+' or '-' to zoom in or out"),
        ])
        .block(Block::bordered());

        f.render_widget(
            paragraph,
            f.area().inner(Margin {
                vertical: 1,
                horizontal: 1,
            }),
        );
    }

    pub fn draw_ui(&mut self, f: &mut Frame, _options: &TopCommand) {
        let all_tabs = WhichTab::all();
        let tab_index = all_tabs
            .iter()
            .position(|&t| t == self.active_tab)
            .unwrap_or(0);
        let tabs = Tabs::new(all_tabs.into_iter().map(|t| t.title())).select(tab_index);
        f.render_widget(tabs, f.area());

        match self.active_tab {
            WhichTab::Series => {
                self.draw_series_ui(f);
            }
            WhichTab::HeatMap => {
                self.draw_heatmap_ui(f);
            }
            WhichTab::Help => {
                self.draw_help_ui(f);
            }
        }

        if !self.error.is_empty() {
            let error_rect = Rect {
                x: 0,
                y: f.area().height.saturating_sub(4),
                width: f.area().width,
                height: 4,
            };

            let status = Paragraph::new(self.error.to_string())
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(Color::Red).bg(Color::Reset));
            f.render_widget(Clear, error_rect);
            f.render_widget(status, error_rect);
        }
    }
}

fn value_with_unit(v: u64, unit: &str) -> String {
    if unit == "b" {
        human_bytes(v as f64)
    } else if unit == "" {
        v.to_formatted_string(&Locale::en)
    } else if unit == "/s" {
        format!("{}/s", v.to_formatted_string(&Locale::en))
    } else if unit == "%" {
        format!("{v:3}%")
    } else if unit == "us" {
        let v = v as f64;
        if v >= 1_000_000.0 {
            format!("{:.3}s", v / 1_000_000.0)
        } else if v >= 1_000.0 {
            format!("{:.0}ms", v / 1_000.0)
        } else {
            format!("{:.0}us", v)
        }
    } else {
        format!("{v}{unit}")
    }
}

pub fn fvalue_with_unit(v: f64, unit: &str) -> String {
    if unit == "b" {
        human_bytes(v)
    } else if unit == "" {
        (v as u64).to_formatted_string(&Locale::en)
    } else if unit == "/s" {
        format!("{}/s", (v as u64).to_formatted_string(&Locale::en))
    } else if unit == "%" {
        format!("{v:3}%")
    } else if unit == "us" {
        if v >= 1_000_000.0 {
            format!("{:.3}s", v / 1_000_000.0)
        } else if v >= 1_000.0 {
            format!("{:.0}ms", v / 1_000.0)
        } else {
            format!("{:.0}us", v)
        }
    } else if unit == "s" {
        let v = v * 1_000_000.0;
        if v >= 1_000_000.0 {
            format!("{}s", v / 1_000_000.0)
        } else if v >= 1_000.0 {
            format!("{:.0}ms", v / 1_000.0)
        } else {
            format!("{:.0}us", v)
        }
    } else {
        format!("{v}{unit}")
    }
}
