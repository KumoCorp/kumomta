use crate::top::accumulator::*;
use crate::top::factories::*;
use crate::top::histogram::*;
use crate::top::state::State;
use crate::top::timeseries::*;
use clap::Parser;
use crossterm::event::{Event, KeyCode, KeyEventKind};
use futures::StreamExt;
use ratatui::prelude::*;
use ratatui::Terminal;
use reqwest::Url;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::time::MissedTickBehavior;

mod accumulator;
mod factories;
mod heatmap;
mod histogram;
mod sparkline;
mod state;
mod timeseries;

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

        state.add_factory(ThreadPoolFactory {});
        state.add_factory(HistogramEventFreqFactory {});
        state.add_factory(HistogramEventAvgFactory {});

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

        state.add_histogram(
            "Inbound SMTP Transaction Duration",
            Histogram::new("smtpsrv_transaction_duration", "s"),
        );

        state.add_histogram(
            "Inbound SMTP Data Receive Latency",
            Histogram::new("smtpsrv_read_data_duration", "s"),
        );

        state.add_histogram(
            "Inbound SMTP Data Process Latency",
            Histogram::new("smtpsrv_process_data_duration", "s"),
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

#[derive(PartialEq)]
enum Action {
    UpdateData,
    Quit,
    Redraw,
    ScrollUp,
    ScrollTop,
    PageUp,
    PageDown,
    ScrollDown,
    ScrollBottom,
    ZoomIn,
    ZoomOut,
    NextTab,
}

#[derive(Default, PartialEq, Copy, Clone)]
enum WhichTab {
    #[default]
    Series,
    HeatMap,
    Help,
}

impl WhichTab {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Series => "Time Series",
            Self::HeatMap => "Heatmaps",
            Self::Help => "Help (press tab to switch tabs)",
        }
    }

    pub fn all() -> Vec<Self> {
        vec![Self::Series, Self::HeatMap, Self::Help]
    }

    pub fn next(&mut self) {
        match self {
            Self::Series => {
                *self = Self::HeatMap;
            }
            Self::HeatMap => {
                *self = Self::Help;
            }
            Self::Help => {
                *self = Self::Series;
            }
        }
    }
}

impl Action {
    fn from_crossterm(event: Event) -> Option<Action> {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
                KeyCode::Down => Some(Action::ScrollDown),
                KeyCode::Up => Some(Action::ScrollUp),
                KeyCode::Home => Some(Action::ScrollTop),
                KeyCode::End => Some(Action::ScrollBottom),
                KeyCode::PageUp => Some(Action::PageUp),
                KeyCode::PageDown => Some(Action::PageDown),
                KeyCode::Char('+') => Some(Action::ZoomIn),
                KeyCode::Char('-') => Some(Action::ZoomOut),
                KeyCode::Tab => Some(Action::NextTab),
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
