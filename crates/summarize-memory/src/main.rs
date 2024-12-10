use clap::Parser;
use human_bytes::human_bytes;
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Summarize memory stats, by focusing the call stacks on the
/// most likely to be interesting frames
#[derive(Parser, Debug)]
struct Opt {
    stats_file: PathBuf,
}

struct RawStatFile {
    summary: String,
    call_stacks: Vec<RawStack>,
}

impl RawStatFile {
    fn new(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let (summary, remainder) = content.split_once("top call stacks:").unwrap();

        let sample_re = Regex::new(
            "^(\\d+) allocations, estimated (\\d+) allocations of (\\d+) total bytes\n",
        )?;

        let mut call_stacks = vec![];
        for entry in remainder.split("sampled every ") {
            let Some(captures) = sample_re.captures(entry) else {
                continue;
            };
            let all = captures.get(0).unwrap().as_str();
            let sampled = captures.get(1).unwrap().as_str().parse()?;
            let count = captures.get(2).unwrap().as_str().parse()?;
            let total_size = captures.get(3).unwrap().as_str().parse()?;

            let stack = entry[all.len()..].to_string();

            call_stacks.push(RawStack {
                sampled,
                count,
                total_size,
                stack,
            })
        }

        println!("Processed {} call stacks", call_stacks.len());

        Ok(Self {
            summary: summary.to_string(),
            call_stacks,
        })
    }
}

struct RawStack {
    sampled: usize,
    count: usize,
    total_size: usize,
    stack: String,
}

impl RawStack {
    fn parse_stack(&self) -> Vec<Frame> {
        let mut frames = vec![];
        let mut lines = self.stack.lines().skip(1).peekable();

        let opt_frame_no_prefix_re = Regex::new("^\\s+(\\d+:\\s+)?").unwrap();
        let at_prefix_re = Regex::new("^\\s+at\\s+").unwrap();

        while let Some(first_line) = lines.next() {
            let Some(m) = opt_frame_no_prefix_re.find(first_line) else {
                break;
            };

            let symbol = first_line[m.len()..].to_string();

            let mut source = String::new();

            if let Some(source_line) = lines.peek() {
                if let Some(m) = at_prefix_re.find(source_line) {
                    source = source_line[m.len()..].to_string();
                    lines.next(); // consume the source line
                }
            }

            frames.push(Frame { symbol, source });
        }

        frames
    }

    fn interesting_stack(&self) -> Vec<Frame> {
        let mut frames = self.parse_stack();

        frames.retain(|frame| match frame.module() {
            "std"
            | "alloc"
            | "core"
            | "tokio"
            | "clone"
            | "__rust_alloc"
            | "__rust_realloc"
            | "tracing"
            | "lua"
            | "hashbrown"
            | "mlua"
            | "serde_json"
            | "toml"
            | "toml_edit"
            | "serde_path_to_error"
            | "serde"
            | "mlua_sys"
            | "ordermap"
            | "indexmap"
            | "sharded_slab"
            | "start_thread"
            | "re_memory"
            | "regex_automata" => false,
            _ => true,
        });

        frames
    }
}

#[derive(Debug, Hash, PartialEq, Eq)]
struct Frame {
    symbol: String,
    source: String,
}

impl Frame {
    fn module(&self) -> &str {
        if self.source.contains("lua-src") {
            return "lua";
        }
        let re = Regex::new("^<*([a-zA-Z_]+)").unwrap();
        re.captures(&self.symbol)
            .and_then(|c| c.get(1))
            .map(|c| c.as_str())
            .unwrap_or("")
    }
}

fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    let stats = RawStatFile::new(&opts.stats_file)?;

    let mut unique_stacks: HashMap<Vec<Frame>, RawStack> = HashMap::new();

    for stack in stats.call_stacks {
        let frames = stack.interesting_stack();

        if let Some(entry) = unique_stacks.get_mut(&frames) {
            entry.total_size += stack.total_size;
            entry.count += stack.count;
        } else {
            unique_stacks.insert(frames, stack);
        }
    }

    let mut stacks: Vec<(RawStack, Vec<Frame>)> = unique_stacks
        .into_iter()
        .map(|(frames, raw)| (raw, frames))
        .collect();
    println!("Aggregated into {} stacks", stacks.len());

    stacks.sort_by(|a, b| b.0.total_size.cmp(&a.0.total_size));

    let interesting_symbols = [
        (
            "kumo_api_types::shaping::Shaping::merge_files::{{closure}}",
            "load shaping data",
        ),
        ("kumod::ready_queue::Fifo::new", "ready queues"),
    ];

    let mut notable_things: HashMap<&str, usize> = HashMap::new();
    for (stack, frames) in &stacks {
        for (symbol, label) in &interesting_symbols {
            for f in frames {
                if f.symbol == *symbol {
                    let entry = notable_things.entry(label).or_insert(0);
                    *entry += stack.total_size;
                    break;
                }
            }
        }
    }

    for (label, size) in notable_things {
        println!("{label}: {}", human_bytes(size as f64));
    }

    for (stack, frames) in stacks {
        println!(
            "{total} in {count} allocations ({per_alloc} each)",
            total = human_bytes(stack.total_size as f64),
            count = stack.count,
            per_alloc = human_bytes(stack.total_size as f64 / stack.count as f64)
        );

        for f in &frames {
            println!("{} {}", f.symbol, f.source);
        }
        println!();
    }

    Ok(())
}
