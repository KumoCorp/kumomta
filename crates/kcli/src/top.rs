use clap::Parser;
use crossterm::event::{Event, KeyCode, KeyEventKind};
use futures::StreamExt;
use human_bytes::human_bytes;
use num_format::{Locale, ToFormattedString};
use ratatui::prelude::*;
use ratatui::symbols::bar::NINE_LEVELS;
use ratatui::widgets::{Block, Borders, Paragraph, RenderDirection, WidgetRef, Wrap};
use ratatui::Terminal;
use reqwest::Url;
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

    diff_state: Option<DiffState>,

    scheduled: Vec<u64>,
    ready: Vec<u64>,
    memory: Vec<u64>,
    error: String,
}

struct DiffState {
    when: Instant,
    delivered: f64,
    received: f64,
    transfail: f64,
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
                    received: metrics
                        .raw
                        .total_messages_received
                        .as_ref()
                        .and_then(|m| m.value.service.get("esmtp_listener"))
                        .copied()
                        .unwrap_or(0.),
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

                if let Some(prior) = self.diff_state.take() {
                    let elapsed = prior.when.elapsed().as_secs_f64();

                    // Compute msgs/s
                    let delivered = (new_state.delivered - prior.delivered) / elapsed;
                    let transfail = (new_state.transfail - prior.transfail) / elapsed;
                    let received = (new_state.received - prior.received) / elapsed;

                    // and add to historical data
                    push_value(&mut self.delivered, delivered as u64);
                    push_value(&mut self.transfail, transfail as u64);
                    push_value(&mut self.received, received as u64);
                }
                self.diff_state.replace(new_state);
            }
            Err(err) => {
                self.error = format!("{err:#}");
                self.diff_state.take();
                push_value(&mut self.message_count, 0);
                push_value(&mut self.message_data_resident, 0);
                push_value(&mut self.scheduled, 0);
                push_value(&mut self.ready, 0);
                push_value(&mut self.listener_conns, 0);
                push_value(&mut self.smtp_conns, 0);
                push_value(&mut self.delivered, 0);
                push_value(&mut self.transfail, 0);
                push_value(&mut self.received, 0);
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
        let sparklines = [
            ("Delivered", &self.delivered, Color::Green, false, "/s"),
            ("Received", &self.received, Color::LightGreen, true, "/s"),
            ("Transfail", &self.transfail, Color::Red, false, "/s"),
            ("Scheduled", &self.scheduled, Color::Yellow, false, ""),
            ("Ready", &self.ready, Color::Gray, false, ""),
            ("Messages", &self.message_count, Color::Blue, false, ""),
            (
                "Resident",
                &self.message_data_resident,
                Color::LightBlue,
                true,
                "",
            ),
            ("Memory", &self.memory, Color::Reset, false, "b"),
            ("Conn Out", &self.smtp_conns, Color::DarkGray, false, ""),
            ("Conn In", &self.listener_conns, Color::Gray, true, ""),
        ];

        let top_bottom = Layout::vertical([
            Constraint::Length(sparklines.len() as u16 * 2),
            Constraint::Fill(1),
        ])
        .split(f.size());

        let throughput_layout =
            Layout::horizontal([Constraint::Fill(1), Constraint::Length(12)]).split(top_bottom[0]);

        let spark_chunks = Layout::vertical(sparklines.iter().map(|_| Constraint::Length(2)))
            .split(throughput_layout[0]);

        let throughput_summary = Layout::vertical(sparklines.iter().map(|_| Constraint::Length(2)))
            .split(throughput_layout[1]);

        for (idx, (label, data, color, inverted, unit)) in sparklines.into_iter().enumerate() {
            let sparkline = Sparkline::default()
                .block(Block::new().borders(Borders::RIGHT))
                .data(data)
                .direction(RenderDirection::RightToLeft)
                .inverted(inverted)
                .style(Style::default().fg(color));
            f.render_widget(sparkline, spark_chunks[idx]);

            let label = Paragraph::new(
                data.get(0)
                    .map(|v| {
                        if unit == "b" {
                            human_bytes(*v as f64)
                        } else if unit == "" {
                            v.to_formatted_string(&Locale::en)
                        } else if unit == "/s" {
                            format!("{}/s", v.to_formatted_string(&Locale::en))
                        } else {
                            format!("{v}{unit}")
                        }
                    })
                    .unwrap_or_else(String::new),
            )
            .right_aligned()
            .block(Block::new().title(label).title_alignment(Alignment::Left));
            f.render_widget(label, throughput_summary[idx]);
        }

        let status = Paragraph::new(self.error.to_string())
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(Color::Red));
        f.render_widget(status, top_bottom[1]);
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
    pub const fn max(mut self, max: u64) -> Self {
        self.max = Some(max);
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
    use ratatui::assert_buffer_eq;
    use ratatui::buffer::Cell;

    #[test]
    fn it_draws() {
        let widget = Sparkline::default().data(&[0, 1, 2, 3, 4, 5, 6, 7, 8]);
        let buffer = render(&widget, 12, 1);
        assert_buffer_eq!(buffer, Buffer::with_lines(vec![" ▁▂▃▄▅▆▇█xxx"]));

        let buffer = render(&widget, 12, 2);
        assert_buffer_eq!(
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
        assert_buffer_eq!(buffer, reversed(Buffer::with_lines(vec!["█▇▆▅▄▃▂▁ "])));

        let buffer = render(&widget, 9, 2);
        assert_buffer_eq!(
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
