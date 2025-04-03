use crate::queue_summary::get_metrics;
use crate::top::sparkline::Sparkline;
use crate::top::timeseries::*;
use crate::top::{Action, TopCommand};
use human_bytes::human_bytes;
use kumo_prometheus::parser::Metric;
use num_format::{Locale, ToFormattedString};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, RenderDirection, Wrap};
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
            .map(|v| {
                if self.unit == "b" {
                    human_bytes(*v as f64)
                } else if self.unit == "" {
                    v.to_formatted_string(&Locale::en)
                } else if self.unit == "/s" {
                    format!("{}/s", v.to_formatted_string(&Locale::en))
                } else if self.unit == "%" {
                    format!("{v:3}%")
                } else if self.unit == "us" {
                    let v = *v as f64;
                    if v >= 1_000_000.0 {
                        format!("{:.3}s", v / 1_000_000.0)
                    } else if v >= 1_000.0 {
                        format!("{:.0}ms", v / 1_000.0)
                    } else {
                        format!("{:.0}us", v)
                    }
                } else {
                    format!("{v}{}", self.unit)
                }
            })
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
    factories: Vec<Box<dyn SeriesFactory + 'static>>,
    error: String,
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
            Action::UpdateData => self.update_metrics(endpoint).await?,
            Action::Redraw => {}
        }
        Ok(())
    }

    pub fn draw_ui(&self, f: &mut Frame, _options: &TopCommand) {
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

        // Figure out the layout; first see if the ideal heights will fit
        let mut base_height = sparklines
            .iter()
            .map(|entry| entry.base_height)
            .sum::<u16>();
        let available_height = f.size().height;

        'adapted_larger: while base_height < available_height {
            // We have room to expand
            for entry in &mut sparklines {
                entry.base_height += 1;
                base_height += 1;

                if base_height == available_height {
                    break 'adapted_larger;
                }
            }
        }

        'adapted_smaller: while base_height > available_height {
            // We need to reduce some row(s)
            let mut progress = false;
            for entry in sparklines.iter_mut().rev() {
                if entry.base_height > 1 {
                    entry.base_height -= 1;
                    base_height -= 1;
                    progress = true;

                    if base_height == available_height {
                        break 'adapted_smaller;
                    }
                }
            }
            if !progress {
                break;
            }
        }

        let label_col_width = sparklines
            .iter()
            .map(|entry| entry.min_width())
            .max()
            .unwrap_or(12);

        let mut y = 0;
        for entry in sparklines.into_iter() {
            if y >= f.size().height {
                break;
            }

            let spark_chunk = Rect {
                x: 0,
                y,
                width: f.size().width - label_col_width,
                height: entry.base_height,
            };
            let summary = Rect {
                x: f.size().width - label_col_width,
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

        if !self.error.is_empty() {
            let error_rect = Rect {
                x: 0,
                y: f.size().height.saturating_sub(4),
                width: f.size().width,
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
