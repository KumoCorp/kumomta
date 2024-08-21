use clap::Parser;
use crossterm::event::{Event, KeyCode, KeyEventKind};
use futures::StreamExt;
use human_bytes::human_bytes;
use num_format::{Locale, ToFormattedString};
use ratatui::prelude::*;
use ratatui::symbols::bar::NINE_LEVELS;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, RenderDirection, WidgetRef, Wrap};
use ratatui::Terminal;
use reqwest::Url;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::time::Instant;

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
        let mut state = State::default();

        loop {
            t.draw(|f| {
                state.draw_ui(f, self);
            })?;

            if let Some(action) = rx.recv().await {
                if action == Action::Quit {
                    return Ok(());
                }
                state.update(action, endpoint).await?;
            }
        }
    }

    async fn spawn_ticker(&self) -> anyhow::Result<UnboundedReceiver<Action>> {
        let (tx, rx) = unbounded_channel();

        let mut stream = crossterm::event::EventStream::new();

        let update_interval = Duration::from_secs(self.update_interval);

        tokio::spawn(async move {
            let mut next_update = Instant::now();
            loop {
                let event = tokio::select! {
                    event = stream.next() => {
                        match event {
                            Some(Ok(event)) => {
                                match Action::from_crossterm(event) {
                                    Some(event) => event,
                                    None => continue,
                                }
                            },
                            _ => Action::Quit,
                        }
                    }
                    _ = tokio::time::sleep_until(next_update) => {
                        next_update = Instant::now() + update_interval;
                        Action::UpdateData
                    }
                };

                if tx.send(event).is_err() {
                    break;
                }
            }
        });

        Ok(rx)
    }
}

fn push_value(target: &mut Vec<u64>, value: u64) {
    target.insert(0, value);
    target.truncate(1024);
}

#[derive(Default)]
struct State {
    message_count: Vec<u64>,
    message_data_resident: Vec<u64>,
    message_meta_resident: Vec<u64>,
    listener_conns: Vec<u64>,
    smtp_conns: Vec<u64>,

    received: Vec<u64>,
    delivered: Vec<u64>,
    transfail: Vec<u64>,
    fail: Vec<u64>,

    diff_state: Option<DiffState>,

    scheduled: Vec<u64>,
    ready: Vec<u64>,
    memory: Vec<u64>,
    error: String,

    thread_pools: BTreeMap<String, Vec<u64>>,
    latency_avg: BTreeMap<String, Vec<u64>>,
    latency_p90: BTreeMap<String, Vec<u64>>,
    latency_count: BTreeMap<String, Vec<u64>>,
}

struct DiffState {
    when: Instant,
    delivered: f64,
    received: f64,
    transfail: f64,
    fail: f64,
    latency: BTreeMap<String, f64>,
}

impl State {
    async fn update_metrics(&mut self, endpoint: &Url) -> anyhow::Result<()> {
        match crate::queue_summary::obtain_metrics(endpoint, true).await {
            Ok(metrics) => {
                self.error.clear();

                for (target, source) in &mut [
                    (&mut self.message_count, &metrics.raw.message_count),
                    (
                        &mut self.message_data_resident,
                        &metrics.raw.message_data_resident_count,
                    ),
                    (
                        &mut self.message_meta_resident,
                        &metrics.raw.message_meta_resident_count,
                    ),
                    (&mut self.memory, &metrics.raw.memory_usage),
                ] {
                    push_value(target, source.as_ref().map_or(0, |m| m.value as u64));
                    target.truncate(1024);
                }

                let new_state = DiffState {
                    when: Instant::now(),
                    delivered: metrics
                        .raw
                        .total_messages_delivered
                        .as_ref()
                        .and_then(|m| m.value.service.get("smtp_client"))
                        .copied()
                        .unwrap_or(0.),
                    transfail: metrics
                        .raw
                        .total_messages_transfail
                        .as_ref()
                        .and_then(|m| m.value.service.get("smtp_client"))
                        .copied()
                        .unwrap_or(0.),
                    fail: metrics
                        .raw
                        .total_messages_fail
                        .as_ref()
                        .and_then(|m| m.value.service.get("smtp_client"))
                        .copied()
                        .unwrap_or(0.),
                    received: metrics
                        .raw
                        .total_messages_received
                        .as_ref()
                        .map(|m| {
                            m.value.service.get("esmtp_listener").copied().unwrap_or(0.)
                                + m.value.service.get("http_listener").copied().unwrap_or(0.)
                        })
                        .unwrap_or(0.),
                    latency: metrics
                        .latency
                        .iter()
                        .map(|entry| (entry.name.to_string(), entry.count as f64))
                        .collect(),
                };
                let scheduled = metrics
                    .raw
                    .scheduled_count
                    .as_ref()
                    .map(|m| m.value.queue.values().copied().sum())
                    .unwrap_or(0.);
                let ready = metrics
                    .raw
                    .ready_count
                    .as_ref()
                    .map(|m| m.value.service.values().copied().sum())
                    .unwrap_or(0.);
                let listener_conns = metrics
                    .raw
                    .connection_count
                    .as_ref()
                    .and_then(|g| g.value.service.get("esmtp_listener").copied())
                    .unwrap_or(0.);
                let smtp_conns = metrics
                    .raw
                    .connection_count
                    .as_ref()
                    .and_then(|g| g.value.service.get("smtp_client").copied())
                    .unwrap_or(0.);

                for (target, value) in [
                    (&mut self.scheduled, scheduled),
                    (&mut self.ready, ready),
                    (&mut self.listener_conns, listener_conns),
                    (&mut self.smtp_conns, smtp_conns),
                ] {
                    push_value(target, value as u64);
                }

                for event in &metrics.latency {
                    let avg_entry = self
                        .latency_avg
                        .entry(event.name.to_string())
                        .or_insert_with(Vec::new);
                    push_value(avg_entry, (event.avg * 1_000_000.0).ceil() as u64);

                    let p90_entry = self
                        .latency_p90
                        .entry(event.name.to_string())
                        .or_insert_with(Vec::new);
                    push_value(p90_entry, (event.p90 * 1_000_000.0).ceil() as u64);
                }

                for pool in &metrics.thread_pools {
                    let entry = self
                        .thread_pools
                        .entry(pool.name.to_string())
                        .or_insert_with(Vec::new);
                    let utilization_percent = (100 * (pool.size - pool.parked)) / pool.size;
                    push_value(entry, utilization_percent as u64);
                }
                let mut dead_pools = vec![];
                for (key, entry) in self.thread_pools.iter_mut() {
                    if metrics
                        .thread_pools
                        .iter()
                        .find(|entry| entry.name == *key)
                        .is_none()
                    {
                        // The pool has gone away.
                        // This can happen for eg: the spoolin pool once
                        // it has completed its work.
                        // We'll treat this as clocking a 0 through.
                        // Once all the data is zero, we'll remove it
                        push_value(entry, 0);
                        if entry.iter().sum::<u64>() == 0 {
                            dead_pools.push(key.to_string());
                        }
                    }
                }
                // Remove any dead thread pools
                for name in dead_pools {
                    self.thread_pools.remove(&name);
                }

                if let Some(prior) = self.diff_state.take() {
                    let elapsed = prior.when.elapsed().as_secs_f64();

                    // Compute msgs/s
                    let delivered = (new_state.delivered - prior.delivered) / elapsed;
                    let transfail = (new_state.transfail - prior.transfail) / elapsed;
                    let fail = (new_state.fail - prior.fail) / elapsed;
                    let received = (new_state.received - prior.received) / elapsed;

                    // and add to historical data
                    push_value(&mut self.delivered, delivered as u64);
                    push_value(&mut self.transfail, transfail as u64);
                    push_value(&mut self.fail, fail as u64);
                    push_value(&mut self.received, received as u64);

                    for (name, new_value) in &new_state.latency {
                        let rate =
                            (new_value - prior.latency.get(name).copied().unwrap_or(0.0)) / elapsed;
                        let entry = self
                            .latency_count
                            .entry(name.to_string())
                            .or_insert_with(Vec::new);
                        push_value(entry, rate as u64);
                    }
                }
                self.diff_state.replace(new_state);
            }
            Err(err) => {
                self.error = format!("{err:#}");
                self.diff_state.take();
                push_value(&mut self.memory, 0);
                push_value(&mut self.message_count, 0);
                push_value(&mut self.message_data_resident, 0);
                push_value(&mut self.scheduled, 0);
                push_value(&mut self.ready, 0);
                push_value(&mut self.listener_conns, 0);
                push_value(&mut self.smtp_conns, 0);
                push_value(&mut self.delivered, 0);
                push_value(&mut self.transfail, 0);
                push_value(&mut self.fail, 0);
                push_value(&mut self.received, 0);
                for target in self.thread_pools.values_mut() {
                    push_value(target, 0);
                }
                for target in self.latency_avg.values_mut() {
                    push_value(target, 0);
                }
                for target in self.latency_p90.values_mut() {
                    push_value(target, 0);
                }
                for target in self.latency_count.values_mut() {
                    push_value(target, 0);
                }
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
            Entry::new("Delivered", &self.delivered, Color::Green, false, "/s", 2),
            Entry::new("Received", &self.received, Color::LightGreen, true, "/s", 2),
            Entry::new("Transfail", &self.transfail, Color::Red, false, "/s", 2),
            Entry::new("Permfail", &self.fail, Color::LightRed, false, "/s", 2),
            Entry::new("Scheduled", &self.scheduled, Color::Green, false, "", 2),
            Entry::new("Ready", &self.ready, Color::LightGreen, false, "", 2),
            Entry::new("Messages", &self.message_count, Color::Green, false, "", 2),
            Entry::new(
                "Resident",
                &self.message_data_resident,
                Color::LightGreen,
                true,
                "",
                1,
            ),
            Entry::new("Memory", &self.memory, Color::Green, false, "b", 2),
            Entry::new(
                "Conn Out",
                &self.smtp_conns,
                Color::LightGreen,
                false,
                "",
                2,
            ),
            Entry::new("Conn In", &self.listener_conns, Color::Green, true, "", 2),
        ];

        let pool_colors = [Color::LightGreen, Color::Green];

        for (pool, data) in self.thread_pools.iter() {
            let next_idx = sparklines.len();
            sparklines.push(Entry::new(
                pool,
                data,
                pool_colors[next_idx % pool_colors.len()],
                false,
                "%",
                1,
            ));
        }
        for (name, data) in self.latency_p90.iter() {
            if name == "init" || name == "pre_init" {
                continue;
            }
            let next_idx = sparklines.len();
            sparklines.push(Entry::new(
                name,
                data,
                pool_colors[next_idx % pool_colors.len()],
                false,
                "us",
                1,
            ));
        }
        for (name, data) in self.latency_count.iter() {
            if name == "init" || name == "pre_init" {
                continue;
            }
            let next_idx = sparklines.len();
            sparklines.push(Entry::new(
                name,
                data,
                pool_colors[next_idx % pool_colors.len()],
                false,
                "/s",
                1,
            ));
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
