use nom::error::{ContextError, ErrorKind};
use nom_locate::LocatedSpan;
use std::fmt::{Debug, Write};

pub(crate) type Span<'a> = LocatedSpan<&'a str>;
pub(crate) type IResult<'a, A, B> = nom::IResult<A, B, ParseError<Span<'a>>>;

pub fn make_span(s: &str) -> Span {
    Span::new(s)
}

#[derive(Debug)]
pub enum ParseErrorKind {
    Context(&'static str),
    Char(char),
    Nom(ErrorKind),
    External { kind: ErrorKind, reason: String },
}

#[derive(Debug)]
pub struct ParseError<I: Debug> {
    pub errors: Vec<(I, ParseErrorKind)>,
}

impl<I: Debug> ContextError<I> for ParseError<I> {
    fn add_context(input: I, ctx: &'static str, mut other: Self) -> Self {
        other.errors.push((input, ParseErrorKind::Context(ctx)));
        other
    }
}

impl<I: Debug> nom::error::ParseError<I> for ParseError<I> {
    fn from_error_kind(input: I, kind: ErrorKind) -> Self {
        Self {
            errors: vec![(input, ParseErrorKind::Nom(kind))],
        }
    }

    fn append(input: I, kind: ErrorKind, mut other: Self) -> Self {
        other.errors.push((input, ParseErrorKind::Nom(kind)));
        other
    }

    fn from_char(input: I, c: char) -> Self {
        Self {
            errors: vec![(input, ParseErrorKind::Char(c))],
        }
    }
}

impl<I: Debug, E: std::fmt::Display> nom::error::FromExternalError<I, E> for ParseError<I> {
    fn from_external_error(input: I, kind: ErrorKind, err: E) -> Self {
        Self {
            errors: vec![(
                input,
                ParseErrorKind::External {
                    kind,
                    reason: format!("{err:#}"),
                },
            )],
        }
    }
}

pub fn make_context_error<'a, S: Into<String>>(
    input: Span<'a>,
    reason: S,
) -> nom::Err<ParseError<Span<'a>>> {
    nom::Err::Error(ParseError {
        errors: vec![(
            input,
            ParseErrorKind::External {
                kind: nom::error::ErrorKind::Fail,
                reason: reason.into(),
            },
        )],
    })
}

pub fn explain_nom(input: Span, err: nom::Err<ParseError<Span<'_>>>) -> String {
    match err {
        nom::Err::Error(e) => {
            let mut result = String::new();
            for (i, (span, kind)) in e.errors.iter().enumerate() {
                if input.is_empty() {
                    match kind {
                        ParseErrorKind::Char(c) => {
                            write!(&mut result, "{i}: expected '{c}', got empty input\n\n")
                        }
                        ParseErrorKind::Context(s) => {
                            write!(&mut result, "{i}: in {s}, got empty input\n\n")
                        }
                        ParseErrorKind::External { kind, reason } => {
                            write!(&mut result, "{i}: {reason} {kind:?}, got empty input\n\n")
                        }
                        ParseErrorKind::Nom(e) => {
                            write!(&mut result, "{i}: in {e:?}, got empty input\n\n")
                        }
                    }
                    .ok();
                    continue;
                }

                let line_number = span.location_line();
                let line = std::str::from_utf8(span.get_line_beginning())
                    .unwrap_or("<INVALID: line slice is not utf8!>");
                // Remap \t in particular, because it can render as multiple
                // columns and defeat the column number calculation provided
                // by the Span type
                let line: String = line
                    .chars()
                    .map(|c| match c {
                        '\t' => '\u{2409}',
                        '\r' => '\u{240d}',
                        '\n' => '\u{240a}',
                        _ => c,
                    })
                    .collect();
                let column = span.get_utf8_column();
                let mut caret = " ".repeat(column.saturating_sub(1));
                caret.push('^');
                for _ in 1..span.fragment().len() {
                    caret.push('_')
                }

                match kind {
                    ParseErrorKind::Char(expected) => {
                        if let Some(actual) = span.fragment().chars().next() {
                            write!(
                                &mut result,
                                "{i}: at line {line_number}:\n\
                                    {line}\n\
                                    {caret}\n\
                                    expected '{expected}', found {actual}\n\n",
                            )
                        } else {
                            write!(
                                &mut result,
                                "{i}: at line {line_number}:\n\
                                    {line}\n\
                                    {caret}\n\
                                    expected '{expected}', got end of input\n\n",
                            )
                        }
                    }
                    ParseErrorKind::Context(context) => {
                        write!(
                            &mut result,
                            "{i}: at line {line_number}, in {context}:\n\
                                {line}\n\
                                {caret}\n\n",
                        )
                    }
                    ParseErrorKind::External { kind, reason } => {
                        write!(
                            &mut result,
                            "{i}: at line {line_number}, {reason} {kind:?}:\n\
                                {line}\n\
                                {caret}\n\n",
                        )
                    }
                    ParseErrorKind::Nom(nom_err) => {
                        write!(
                            &mut result,
                            "{i}: at line {line_number}, in {nom_err:?}:\n\
                                {line}\n\
                                {caret}\n\n",
                        )
                    }
                }
                .ok();
            }
            result
        }
        _ => format!("{err:#}"),
    }
}
