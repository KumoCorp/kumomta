use bstr::{BStr, ByteSlice};
use nom::error::{ContextError, ErrorKind};
use nom::Input;
use nom_locate::LocatedSpan;
use std::fmt::{Debug, Write};
use std::marker::PhantomData;

pub type Span<'a> = LocatedSpan<&'a [u8]>;
pub type IResult<'a, A, B> = nom::IResult<A, B, ParseError<Span<'a>>>;

pub fn make_span(s: &'_ [u8]) -> Span<'_> {
    Span::new(s)
}

/// Like nom::bytes::complete::tag, except that we print what the tag
/// was expecting if there was an error.
/// I feel like this should be the default behavior TBH.
pub fn tag<E>(tag: &'static str) -> TagParser<E> {
    TagParser {
        tag,
        e: PhantomData,
    }
}

/// Struct to support displaying better errors for tag()
pub struct TagParser<E> {
    tag: &'static str,
    e: PhantomData<E>,
}

/// All this fuss to show what we expected for the TagParser impl
impl<I, Error: nom::error::ParseError<I> + nom::error::FromExternalError<I, String>> nom::Parser<I>
    for TagParser<Error>
where
    I: nom::Input + nom::Compare<&'static str> + nom::AsBytes,
{
    type Output = I;
    type Error = Error;

    fn process<OM: nom::OutputMode>(
        &mut self,
        i: I,
    ) -> nom::PResult<OM, I, Self::Output, Self::Error> {
        use nom::error::ErrorKind;
        use nom::{CompareResult, Err, Mode};

        let tag_len = self.tag.input_len();

        match i.compare(self.tag) {
            CompareResult::Ok => Ok((i.take_from(tag_len), OM::Output::bind(|| i.take(tag_len)))),
            CompareResult::Incomplete => Err(Err::Error(OM::Error::bind(|| {
                Error::from_external_error(
                    i,
                    ErrorKind::Fail,
                    format!(
                        "expected \"{}\" but ran out of input",
                        self.tag.escape_debug()
                    ),
                )
            }))),

            CompareResult::Error => {
                let available = i.take(i.input_len().min(tag_len));
                Err(Err::Error(OM::Error::bind(|| {
                    Error::from_external_error(
                        i,
                        ErrorKind::Fail,
                        format!(
                            "expected \"{}\" but found {:?}",
                            self.tag.escape_debug(),
                            BStr::new(available.as_bytes())
                        ),
                    )
                })))
            }
        }
    }
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

pub fn make_context_error<S: Into<String>>(
    input: Span<'_>,
    reason: S,
) -> nom::Err<ParseError<Span<'_>>> {
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
            let mut lines_shown = vec![];

            for (span, kind) in e.errors.iter() {
                if input.is_empty() {
                    match kind {
                        ParseErrorKind::Char(c) => {
                            write!(&mut result, "Error expected '{c}', got empty input\n\n")
                        }
                        ParseErrorKind::Context(s) => {
                            write!(&mut result, "Error in {s}, got empty input\n\n")
                        }
                        ParseErrorKind::External { kind, reason } => {
                            write!(&mut result, "Error {reason} {kind:?}, got empty input\n\n")
                        }
                        ParseErrorKind::Nom(e) => {
                            write!(&mut result, "Error in {e:?}, got empty input\n\n")
                        }
                    }
                    .ok();
                    continue;
                }

                let line_number = span.location_line();
                let input_line = span.get_line_beginning();
                // Remap \t in particular, because it can render as multiple
                // columns and defeat the column number calculation provided
                // by the Span type
                let mut line = String::new();
                for (start, end, c) in input_line.char_indices() {
                    let c = match c {
                        '\t' => '\u{2409}',
                        '\r' => '\u{240d}',
                        '\n' => '\u{240a}',
                        c => c,
                    };

                    if c == std::char::REPLACEMENT_CHARACTER {
                        let bytes = &input_line[start..end];
                        for b in bytes.iter() {
                            line.push_str(&format!("\\x{b:02X}"));
                        }
                    } else {
                        line.push(c);
                    }
                }

                let column = span.get_utf8_column();

                lines_shown.push(line_number);

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
                                "Error at line {line_number}:\n\
                                    {line}\n\
                                    {caret}\n\
                                    expected '{expected}', found {actual}\n\n",
                            )
                        } else {
                            write!(
                                &mut result,
                                "Error at line {line_number}:\n\
                                    {line}\n\
                                    {caret}\n\
                                    expected '{expected}', got end of input\n\n",
                            )
                        }
                    }
                    ParseErrorKind::Context(context) => {
                        write!(&mut result, "while parsing {context}\n")
                    }
                    ParseErrorKind::External { kind: _, reason } => {
                        write!(
                            &mut result,
                            "Error at line {line_number}, {reason}:\n\
                                {line}\n\
                                {caret}\n\n",
                        )
                    }
                    ParseErrorKind::Nom(nom_err) => {
                        write!(
                            &mut result,
                            "Error at line {line_number}, in {nom_err:?}:\n\
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
