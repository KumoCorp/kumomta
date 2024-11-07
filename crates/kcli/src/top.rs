use crate::queue_summary::get_metrics;
use clap::Parser;
use crossterm::event::{Event, KeyCode, KeyEventKind};
use futures::StreamExt;
use human_bytes::human_bytes;
use kumo_prometheus::parser::Metric;
use num_format::{Locale, ToFormattedString};
use ratatui::prelude::*;
use ratatui::symbols::bar::NINE_LEVELS;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, RenderDirection, WidgetRef, Wrap};
use ratatui::Terminal;
use reqwest::Url;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::time::{Instant, MissedTickBehavior};

/// Continually update and show what's happening in kumod
#[derive(Debug, Parser)]
pub struct TopCommand {
    #[arg(long, default_value = "1")]
    update_interval: u64,
}

impl TopCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        initialize_panic_handler();
        startup()?;

        let result = self.run_tui(endpoint).await;

        shutdown()?;

        result
    }

    async fn run_tui(&self, endpoint: &Url) -> anyhow::Result<()> {
        let mut t = Terminal::new(CrosstermBackend::new(std::io::stderr()))?;

        let mut rx = self.spawn_ticker().await?;
        let mut state = State {
            time_series: HashMap::new(),
            factories: vec![],

            error: String::new(),
        };

        state.factories.push(Box::new(ThreadPoolFactory {}));
        state.factories.push(Box::new(HistogramEventFreqFactory {}));
        state.factories.push(Box::new(HistogramEventAvgFactory {}));

        state.add_series(
            "message_count",
            TimeSeries::new(DirectAccumulator::new("message_count")),
        );
        state.add_series(
            "message_data_resident_count",
            TimeSeries::new(DirectAccumulator::new("message_data_resident_count")),
        );
        state.add_series(
            "memory_usage",
            TimeSeries::new(DirectAccumulator::new("memory_usage")),
        );
        state.add_series(
            "scheduled_count_total",
            TimeSeries::new(DirectAccumulator::new("scheduled_count_total")),
        );
        state.add_series(
            "ready_count",
            TimeSeries::new(SummingAccumulator::new("ready_count", |metric| {
                // Only include metrics like `smtp_client:something`.
                // Don't include `smtp_client` because that is already
                // a sum.  Alternatively, we could only include the summed
                // metrics. It doesn't matter here because we have both
                // sets of data, we just need to pick one of them so
                // that we don't double count.
                metric
                    .labels()
                    .get("service")
                    .map(|s| s.contains(':'))
                    .unwrap_or(false)
            })),
        );
        state.add_series(
            "total_messages_delivered_rate",
            TimeSeries::new(RateAccumulator::new(
                DirectAccumulator::new_with_label_match(
                    "total_messages_delivered",
                    "service",
                    "smtp_client",
                ),
            )),
        );
        state.add_series(
            "total_messages_transfail_rate",
            TimeSeries::new(RateAccumulator::new(
                DirectAccumulator::new_with_label_match(
                    "total_messages_transfail",
                    "service",
                    "smtp_client",
                ),
            )),
        );
        state.add_series(
            "total_messages_fail_rate",
            TimeSeries::new(RateAccumulator::new(
                DirectAccumulator::new_with_label_match(
                    "total_messages_fail",
                    "service",
                    "smtp_client",
                ),
            )),
        );
        state.add_series(
            "total_messages_received_rate",
            TimeSeries::new(RateAccumulator::new(SumMultipleAccumulator::new(vec![
                Box::new(DirectAccumulator::new_with_label_match(
                    "total_messages_received",
                    "service",
                    "esmtp_listener",
                )),
                Box::new(DirectAccumulator::new_with_label_match(
                    "total_messages_received",
                    "service",
                    "http_listener",
                )),
            ]))),
        );
        state.add_series(
            "listener_conns",
            TimeSeries::new(DirectAccumulator::new_with_label_match(
                "connection_count",
                "service",
                "esmtp_listener",
            )),
        );
        state.add_series(
            "smtp_conns",
            TimeSeries::new(DirectAccumulator::new_with_label_match(
                "connection_count",
                "service",
                "smtp_client",
            )),
        );

        let mut ticker = tokio::time::interval(Duration::from_secs(self.update_interval));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            t.draw(|f| {
                state.draw_ui(f, self);
            })?;

            tokio::select! {
                action = rx.recv() => {
                    if let Some(action) = action {
                        if action == Action::Quit {
                            return Ok(());
                        }
                        state.update(action, endpoint).await?;
                    }
                }
                _ = ticker.tick() => {
                    state.update(Action::UpdateData, endpoint).await?;
                }
            }
        }
    }

    async fn spawn_ticker(&self) -> anyhow::Result<UnboundedReceiver<Action>> {
        let (tx, rx) = unbounded_channel();

        let mut stream = crossterm::event::EventStream::new();

        tokio::spawn(async move {
            loop {
                let event = stream.next().await;
                let event = match event {
                    Some(Ok(event)) => match Action::from_crossterm(event) {
                        Some(event) => event,
                        None => continue,
                    },
                    _ => Action::Quit,
                };

                if tx.send(event).is_err() {
                    break;
                }
            }
        });

        Ok(rx)
    }
}

trait SeriesFactory {
    /// Returns the series name that should be created for this metric
    fn matches(&self, metric: &Metric) -> Option<String>;
    /// Constructs the appropriate series for this metric
    fn factory(&self, series_name: &str, metric: &Metric) -> TimeSeries;
}

trait Accumulator {
    fn accumulate(&mut self, metric: &Metric);
    fn commit(&mut self) -> f64;
}

struct SeriesChartOptions {
    name: String,
    inverted: bool,
    unit: String,
}

struct TimeSeries {
    data: Vec<u64>,
    accumulator: Box<dyn Accumulator + 'static>,
    chart: Option<SeriesChartOptions>,
}

impl TimeSeries {
    fn new<A: Accumulator + 'static>(accumulator: A) -> Self {
        Self {
            data: vec![],
            accumulator: Box::new(accumulator),
            chart: None,
        }
    }

    fn set_chart(&mut self, chart: SeriesChartOptions) {
        self.chart.replace(chart);
    }

    fn accumulate(&mut self, metric: &Metric) {
        self.accumulator.accumulate(metric);
    }

    fn commit(&mut self) {
        let value = self.accumulator.commit();
        self.data.insert(0, value as u64);
        self.data.truncate(1024);
    }
}

#[allow(unused)]
enum AccumulatorTarget {
    Value,
    HistogramCount,
    HistogramSum,
    HistogramQuantile(f64),
}
impl AccumulatorTarget {
    fn get_value(&self, metric: &Metric) -> f64 {
        match self {
            AccumulatorTarget::Value => metric.value(),
            AccumulatorTarget::HistogramCount => metric.as_histogram().count,
            AccumulatorTarget::HistogramSum => metric.as_histogram().sum,
            AccumulatorTarget::HistogramQuantile(n) => metric.as_histogram().quantile(*n),
        }
    }
}

struct DirectAccumulator {
    name: String,
    label: Option<String>,
    label_value: Option<String>,
    value: Option<f64>,
    target: AccumulatorTarget,
    scale: f64,
}

impl DirectAccumulator {
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            label: None,
            label_value: None,
            value: None,
            target: AccumulatorTarget::Value,
            scale: 1.0,
        }
    }

    pub fn new_with_label_match<N: Into<String>, L: Into<String>, V: Into<String>>(
        name: N,
        label: L,
        label_value: V,
    ) -> Self {
        Self {
            name: name.into(),
            label: Some(label.into()),
            label_value: Some(label_value.into()),
            value: None,
            target: AccumulatorTarget::Value,
            scale: 1.0,
        }
    }

    pub fn set_target(&mut self, target: AccumulatorTarget) {
        self.target = target;
    }

    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
    }
}

impl Accumulator for DirectAccumulator {
    fn accumulate(&mut self, metric: &Metric) {
        if metric.name().as_str() == self.name.as_str() {
            match (&self.label, &self.label_value) {
                (Some(label_name), Some(label_value)) => {
                    if metric.label_is(label_name, label_value) {
                        self.value
                            .replace(self.target.get_value(metric) * self.scale);
                    }
                }
                _ => {
                    self.value
                        .replace(self.target.get_value(metric) * self.scale);
                }
            }
        }
    }

    fn commit(&mut self) -> f64 {
        self.value.take().unwrap_or(0.0)
    }
}

struct SumMultipleAccumulator {
    accumulators: Vec<Box<dyn Accumulator + 'static>>,
}

impl SumMultipleAccumulator {
    pub fn new(accumulators: Vec<Box<dyn Accumulator + 'static>>) -> Self {
        Self { accumulators }
    }
}

impl Accumulator for SumMultipleAccumulator {
    fn accumulate(&mut self, metric: &Metric) {
        for a in &mut self.accumulators {
            a.accumulate(metric);
        }
    }
    fn commit(&mut self) -> f64 {
        let mut result = 0.0;
        for a in &mut self.accumulators {
            result += a.commit();
        }
        result
    }
}

struct SummingAccumulator {
    name: String,
    value: Option<f64>,
    filter: Box<dyn Fn(&Metric) -> bool>,
}

impl SummingAccumulator {
    pub fn new<S: Into<String>, F: Fn(&Metric) -> bool + 'static>(name: S, filter: F) -> Self {
        Self {
            name: name.into(),
            value: None,
            filter: Box::new(filter),
        }
    }
}

impl Accumulator for SummingAccumulator {
    fn accumulate(&mut self, metric: &Metric) {
        if metric.name().as_str() == self.name.as_str() {
            if !(self.filter)(metric) {
                return;
            }
            let value = self.value.take().unwrap_or(0.) + metric.value();
            self.value.replace(value);
        }
    }

    fn commit(&mut self) -> f64 {
        self.value.take().unwrap_or(0.)
    }
}

struct RateAccumulator {
    accumulator: Box<dyn Accumulator + 'static>,
    prior: Option<(Instant, f64)>,
}

impl RateAccumulator {
    pub fn new<A: Accumulator + 'static>(accumulator: A) -> Self {
        Self {
            accumulator: Box::new(accumulator),
            prior: None,
        }
    }
}

impl Accumulator for RateAccumulator {
    fn accumulate(&mut self, metric: &Metric) {
        self.accumulator.accumulate(metric);
    }

    fn commit(&mut self) -> f64 {
        let value = self.accumulator.commit();

        let result = if let Some((when, prior_value)) = self.prior.take() {
            let elapsed = when.elapsed().as_secs_f64();

            (value - prior_value) / elapsed
        } else {
            0.0
        };

        self.prior.replace((Instant::now(), value));

        result
    }
}

struct ThreadPoolAccumulator {
    pool: String,
    size: f64,
    parked: f64,
}

impl ThreadPoolAccumulator {
    fn new(pool: &str) -> Self {
        Self {
            pool: pool.to_string(),
            size: 0.,
            parked: 0.,
        }
    }
}

impl Accumulator for ThreadPoolAccumulator {
    fn accumulate(&mut self, metric: &Metric) {
        match metric.name().as_str() {
            "thread_pool_size" => {
                if metric.label_is("pool", &self.pool) {
                    self.size = metric.value();
                }
            }
            "thread_pool_parked" => {
                if metric.label_is("pool", &self.pool) {
                    self.parked = metric.value();
                }
            }
            _ => {}
        }
    }

    fn commit(&mut self) -> f64 {
        if self.size == 0.0 {
            0.0
        } else {
            let utilization_percent = (100. * (self.size - self.parked)) / self.size;
            self.parked = self.size;
            self.size = 0.0;
            utilization_percent
        }
    }
}

struct ThreadPoolFactory {}

impl SeriesFactory for ThreadPoolFactory {
    fn matches(&self, metric: &Metric) -> Option<String> {
        match metric.name().as_str() {
            "thread_pool_size" | "thread_pool_parked" => {
                metric.labels().get("pool").map(|s| format!("pool {s}"))
            }
            _ => None,
        }
    }

    fn factory(&self, series_name: &str, metric: &Metric) -> TimeSeries {
        let pool = metric.labels().get("pool").unwrap();
        let mut series = TimeSeries::new(ThreadPoolAccumulator::new(&pool));

        series.set_chart(SeriesChartOptions {
            name: series_name.to_string(),
            inverted: false,
            unit: "%".to_string(),
        });

        series
    }
}

struct HistogramEventFreqFactory {}

impl SeriesFactory for HistogramEventFreqFactory {
    fn matches(&self, metric: &Metric) -> Option<String> {
        if metric.is_histogram() {
            let h = metric.as_histogram();

            let label = match h.labels.values().next() {
                Some(l) => l.as_str(),
                None => h.name.as_str(),
            };

            if label == "init" || label == "pre_init" {
                return None;
            }

            Some(format!("freq: {label}"))
        } else {
            None
        }
    }

    fn factory(&self, series_name: &str, metric: &Metric) -> TimeSeries {
        let h = metric.as_histogram();
        let mut count_series = match h.labels.iter().next() {
            Some((key, value)) => DirectAccumulator::new_with_label_match(
                metric.name().to_string(),
                key.to_string(),
                value.to_string(),
            ),
            None => DirectAccumulator::new(metric.name().to_string()),
        };
        count_series.set_target(AccumulatorTarget::HistogramCount);

        let mut series = TimeSeries::new(RateAccumulator::new(count_series));

        series.set_chart(SeriesChartOptions {
            name: series_name.to_string(),
            inverted: false,
            unit: "/s".to_string(),
        });

        series
    }
}

struct HistogramEventAvgFactory {}

impl SeriesFactory for HistogramEventAvgFactory {
    fn matches(&self, metric: &Metric) -> Option<String> {
        if metric.is_histogram() {
            let h = metric.as_histogram();

            let label = match h.labels.values().next() {
                Some(l) => l.as_str(),
                None => h.name.as_str(),
            };

            if label == "init" || label == "pre_init" {
                return None;
            }

            Some(format!("avg: {label}"))
        } else {
            None
        }
    }

    fn factory(&self, series_name: &str, metric: &Metric) -> TimeSeries {
        let h = metric.as_histogram();
        let mut count_series = match h.labels.iter().next() {
            Some((key, value)) => DirectAccumulator::new_with_label_match(
                metric.name().to_string(),
                key.to_string(),
                value.to_string(),
            ),
            None => DirectAccumulator::new(metric.name().to_string()),
        };
        // The "Value" of a histogram is sum / count which == avg over its lifetime
        count_series.set_target(AccumulatorTarget::Value);
        count_series.set_scale(1_000_000.0);

        let mut series = TimeSeries::new(count_series);

        series.set_chart(SeriesChartOptions {
            name: series_name.to_string(),
            inverted: false,
            unit: "us".to_string(),
        });

        series
    }
}
struct State {
    time_series: HashMap<String, TimeSeries>,
    factories: Vec<Box<dyn SeriesFactory + 'static>>,
    error: String,
}

impl State {
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

    fn add_series<S: Into<String>>(&mut self, name: S, series: TimeSeries) {
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

    async fn update(&mut self, action: Action, endpoint: &Url) -> anyhow::Result<()> {
        match action {
            Action::Quit => anyhow::bail!("quit!"),
            Action::UpdateData => self.update_metrics(endpoint).await?,
            Action::Redraw => {}
        }
        Ok(())
    }

    fn draw_ui(&self, f: &mut Frame, _options: &TopCommand) {
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

#[derive(PartialEq)]
enum Action {
    UpdateData,
    Quit,
    Redraw,
}

impl Action {
    fn from_crossterm(event: Event) -> Option<Action> {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
                _ => None,
            },
            Event::Key(_) => None,
            Event::FocusGained | Event::FocusLost => None,
            Event::Mouse(_) => None,
            Event::Paste(_) => None,
            Event::Resize(_, _) => Some(Action::Redraw),
        }
    }
}

fn startup() -> anyhow::Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(std::io::stderr(), crossterm::terminal::EnterAlternateScreen)?;
    Ok(())
}

fn shutdown() -> anyhow::Result<()> {
    crossterm::execute!(std::io::stderr(), crossterm::terminal::LeaveAlternateScreen)?;
    crossterm::terminal::disable_raw_mode()?;
    Ok(())
}

fn initialize_panic_handler() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        shutdown().unwrap();
        original_hook(panic_info);
    }));
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct Sparkline<'a> {
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
                buf.get_mut(x, spark_area.top() + j)
                    .set_symbol(symbol)
                    .set_style(self.style());

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
        let mut buffer = Buffer::filled(area, &cell);
        widget.render(area, &mut buffer);
        buffer
    }
}
