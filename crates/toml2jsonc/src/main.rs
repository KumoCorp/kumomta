//! This utility converts a TOML file with comments into
//! a JSON file with comments.
//!
//! Its intended purpose is to demonstrate the JSON representation
//! of TOML data files from the KumoMTA documentation.

use anyhow::Context;
use clap::Parser;
use std::io::Read;
use toml_edit::{Item, Table, Value as TValue};

/// toml2jsonc - Convert TOML files to JSON-with-Comments
#[derive(Debug, Parser)]
#[command(about)]
struct Opt {
    /// The path to the toml file to read.
    /// If omitted, stdin will be consumed
    /// and interpreted as TOML
    #[arg(name = "TOML_FILE")]
    input: Option<String>,
}

impl Opt {
    fn read_input(&self) -> anyhow::Result<String> {
        match &self.input {
            Some(path) => {
                std::fs::read_to_string(&path).context(format!("failed to read file {path}"))
            }
            None => {
                let mut result = String::new();
                std::io::stdin().read_to_string(&mut result)?;
                Ok(result)
            }
        }
    }
}

#[derive(Clone, Debug)]
enum Value {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<CommentedValue>),
    Object(Vec<(String, CommentedValue)>),
}

impl Value {
    fn insert<K: Into<String>, V: Into<CommentedValue>>(&mut self, key: K, value: V) {
        match self {
            Self::Object(array) => {
                array.push((key.into(), value.into()));
            }
            _ => unreachable!(),
        }
    }

    fn push<V: Into<CommentedValue>>(&mut self, value: V) {
        match self {
            Self::Array(array) => {
                array.push(value.into());
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Clone, Debug)]
struct CommentedValue {
    before: Option<String>,
    value: Value,
    after: Option<String>,
}

fn emit_comment<W: std::fmt::Write>(
    comment: Option<&str>,
    target: &mut W,
    indent: &str,
) -> anyhow::Result<()> {
    if let Some(c) = comment {
        for line in c.lines() {
            write!(target, "{indent}// {line}\n")?;
        }
    }

    Ok(())
}

impl CommentedValue {
    fn pretty_print_to<W: std::fmt::Write>(
        &self,
        target: &mut W,
        depth: usize,
    ) -> anyhow::Result<()> {
        let brace_indent = " ".repeat(depth * 2);
        let indent = " ".repeat((depth + 1) * 2);

        match &self.value {
            Value::Null => target.write_str("null")?,
            Value::Bool(true) => target.write_str("true")?,
            Value::Bool(false) => target.write_str("false")?,
            Value::Number(n) => {
                let s = serde_json::to_string(n)?;
                target.write_str(&s)?;
            }
            Value::String(s) => {
                let s = serde_json::to_string(s)?;
                target.write_str(&s)?;
            }
            Value::Array(arr) => {
                write!(target, "[\n")?;

                let mut iter = arr.iter().peekable();
                while let Some(value) = iter.next() {
                    emit_comment(value.before.as_deref(), target, &indent)?;

                    write!(target, "{indent}")?;
                    value.pretty_print_to(target, depth + 1)?;

                    if iter.peek().is_some() {
                        write!(target, ",")?;
                    }
                    write!(target, "\n")?;

                    emit_comment(value.after.as_deref(), target, &indent)?;
                }

                write!(target, "{brace_indent}]")?;
            }
            Value::Object(map) => {
                if depth == 0 {
                    write!(target, "{brace_indent}{{\n")?;
                } else {
                    write!(target, "{{\n")?;
                }
                let mut iter = map.iter().peekable();
                while let Some((key, value)) = iter.next() {
                    emit_comment(value.before.as_deref(), target, &indent)?;

                    let key_str = serde_json::to_string(key)?;
                    write!(target, "{indent}{key_str}: ")?;

                    value.pretty_print_to(target, depth + 1)?;
                    if iter.peek().is_some() {
                        write!(target, ",")?;
                    }
                    write!(target, "\n")?;

                    emit_comment(value.after.as_deref(), target, &indent)?;
                }
                write!(target, "{brace_indent}}}")?;
            }
        };

        Ok(())
    }
}

impl From<Value> for CommentedValue {
    fn from(value: Value) -> Self {
        Self {
            before: None,
            value,
            after: None,
        }
    }
}

impl From<&TValue> for CommentedValue {
    fn from(value: &TValue) -> Self {
        match value {
            TValue::String(s) => Value::String(s.value().to_string()),
            TValue::Integer(s) => Value::Number((*s.value()).into()),
            TValue::Float(s) => Value::Number(
                serde_json::Number::from_f64(*s.value())
                    .expect("f64 value is not representible in JSON"),
            ),
            TValue::Boolean(s) => Value::Bool(*s.value()),
            TValue::Datetime(s) => Value::String(s.value().to_string()),
            TValue::Array(a) => {
                let mut array = Value::Array(vec![]);
                for v in a.iter() {
                    array.push(v);
                }
                array
            }
            TValue::InlineTable(t) => {
                let mut map = Value::Object(vec![]);
                for (k, v) in t.iter() {
                    map.insert(k, v);
                }
                map
            }
        }
        .into()
    }
}

fn toml_comment_to_json(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }

    let s = if trimmed.contains('\n') {
        trimmed.replace("\n# ", "\n").replace("\n#\n", "\n\n")
    } else {
        trimmed.to_string()
    };

    Some(s.strip_prefix("# ").unwrap_or(&s).to_string())
}

fn map_table(table: &Table) -> CommentedValue {
    let mut obj = Value::Object(vec![]);

    for (k, item) in table.iter() {
        let key = table.key(k).expect("key to have entry");

        let mut value = match item {
            Item::None => CommentedValue::from(Value::Null),
            Item::Value(value) => CommentedValue::from(value),
            Item::Table(table) => map_table(table),
            Item::ArrayOfTables(tables) => {
                let mut value = Value::Array(vec![]);

                for t in tables.iter() {
                    value.push(map_table(t));
                }

                value.into()
            }
        };

        if let Some(comment) = key
            .leaf_decor()
            .prefix()
            .and_then(|d| d.as_str())
            .and_then(toml_comment_to_json)
        {
            value.before.replace(comment);
        }
        if let Some(comment) = key
            .leaf_decor()
            .suffix()
            .and_then(|d| d.as_str())
            .and_then(toml_comment_to_json)
        {
            value.after.replace(comment);
        }

        obj.insert(k, value);
    }

    let mut value = CommentedValue::from(obj);
    if let Some(comment) = table
        .decor()
        .prefix()
        .and_then(|d| d.as_str())
        .and_then(toml_comment_to_json)
    {
        value.before.replace(comment);
    }
    if let Some(comment) = table
        .decor()
        .suffix()
        .and_then(|d| d.as_str())
        .and_then(toml_comment_to_json)
    {
        value.after.replace(comment);
    }
    value
}

fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    let input = opts.read_input()?;
    let toml = input.parse::<toml_edit::DocumentMut>()?;
    let json = map_table(&toml);

    let mut output = String::new();
    json.pretty_print_to(&mut output, 0)?;
    println!("{output}");

    Ok(())
}
