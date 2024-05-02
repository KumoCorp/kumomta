use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq)]
pub struct SmtpClientTimeouts {
    #[serde(
        default = "SmtpClientTimeouts::default_connect_timeout",
        with = "duration_serde"
    )]
    pub connect_timeout: Duration,

    #[serde(
        default = "SmtpClientTimeouts::default_ehlo_timeout",
        with = "duration_serde"
    )]
    pub ehlo_timeout: Duration,

    #[serde(
        default = "SmtpClientTimeouts::default_mail_from_timeout",
        with = "duration_serde"
    )]
    pub mail_from_timeout: Duration,

    #[serde(
        default = "SmtpClientTimeouts::default_rcpt_to_timeout",
        with = "duration_serde"
    )]
    pub rcpt_to_timeout: Duration,

    #[serde(
        default = "SmtpClientTimeouts::default_data_timeout",
        with = "duration_serde"
    )]
    pub data_timeout: Duration,
    #[serde(
        default = "SmtpClientTimeouts::default_data_dot_timeout",
        with = "duration_serde"
    )]
    pub data_dot_timeout: Duration,
    #[serde(
        default = "SmtpClientTimeouts::default_rset_timeout",
        with = "duration_serde"
    )]
    pub rset_timeout: Duration,

    #[serde(
        default = "SmtpClientTimeouts::default_idle_timeout",
        with = "duration_serde"
    )]
    pub idle_timeout: Duration,

    #[serde(
        default = "SmtpClientTimeouts::default_starttls_timeout",
        with = "duration_serde"
    )]
    pub starttls_timeout: Duration,

    #[serde(
        default = "SmtpClientTimeouts::default_auth_timeout",
        with = "duration_serde"
    )]
    pub auth_timeout: Duration,
}

impl Default for SmtpClientTimeouts {
    fn default() -> Self {
        Self {
            connect_timeout: Self::default_connect_timeout(),
            ehlo_timeout: Self::default_ehlo_timeout(),
            mail_from_timeout: Self::default_mail_from_timeout(),
            rcpt_to_timeout: Self::default_rcpt_to_timeout(),
            data_timeout: Self::default_data_timeout(),
            data_dot_timeout: Self::default_data_dot_timeout(),
            rset_timeout: Self::default_rset_timeout(),
            idle_timeout: Self::default_idle_timeout(),
            starttls_timeout: Self::default_starttls_timeout(),
            auth_timeout: Self::default_auth_timeout(),
        }
    }
}

impl SmtpClientTimeouts {
    fn default_connect_timeout() -> Duration {
        Duration::from_secs(60)
    }
    fn default_auth_timeout() -> Duration {
        Duration::from_secs(60)
    }
    fn default_ehlo_timeout() -> Duration {
        Duration::from_secs(300)
    }
    fn default_mail_from_timeout() -> Duration {
        Duration::from_secs(300)
    }
    fn default_rcpt_to_timeout() -> Duration {
        Duration::from_secs(300)
    }
    fn default_data_timeout() -> Duration {
        Duration::from_secs(300)
    }
    fn default_data_dot_timeout() -> Duration {
        Duration::from_secs(300)
    }
    fn default_rset_timeout() -> Duration {
        Duration::from_secs(5)
    }
    fn default_idle_timeout() -> Duration {
        Duration::from_secs(5)
    }
    fn default_starttls_timeout() -> Duration {
        Duration::from_secs(5)
    }

    pub fn short_timeouts() -> Self {
        let short = Duration::from_secs(20);
        Self {
            connect_timeout: short,
            ehlo_timeout: short,
            mail_from_timeout: short,
            rcpt_to_timeout: short,
            data_timeout: short,
            data_dot_timeout: short,
            rset_timeout: short,
            idle_timeout: short,
            starttls_timeout: short,
            auth_timeout: short,
        }
    }

    /// Compute theoretical maximum lifetime of a single message send
    pub fn total_message_send_duration(&self) -> Duration {
        self.connect_timeout
            + self.ehlo_timeout
            + self.auth_timeout
            + self.mail_from_timeout
            + self.rcpt_to_timeout
            + self.data_timeout
            + self.data_dot_timeout
            + self.starttls_timeout
            + self.idle_timeout
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone, Hash)]
pub struct Response {
    pub code: u16,
    pub enhanced_code: Option<EnhancedStatusCode>,
    #[serde(serialize_with = "as_single_line")]
    pub content: String,
    pub command: Option<String>,
}

impl Response {
    pub fn to_single_line(&self) -> String {
        let mut line = format!("{} ", self.code);

        if let Some(enh) = &self.enhanced_code {
            line.push_str(&format!("{}.{}.{} ", enh.class, enh.subject, enh.detail));
        }

        line.push_str(&remove_line_break(&self.content));

        line
    }

    pub fn is_transient(&self) -> bool {
        self.code >= 400 && self.code < 500
    }

    pub fn is_permanent(&self) -> bool {
        self.code >= 500 && self.code < 600
    }

    pub fn with_code_and_message(code: u16, message: &str) -> Self {
        let lines: Vec<&str> = message.lines().collect();

        let mut builder = ResponseBuilder::new(&ResponseLine {
            code,
            content: lines[0],
            is_final: lines.len() == 1,
        });

        for (n, line) in lines.iter().enumerate().skip(1) {
            builder
                .add_line(&ResponseLine {
                    code,
                    content: line,
                    is_final: n == lines.len() - 1,
                })
                .ok();
        }

        builder.build(None)
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone, Copy, Hash)]
pub struct EnhancedStatusCode {
    pub class: u8,
    pub subject: u16,
    pub detail: u16,
}

fn parse_enhanced_status_code(line: &str) -> Option<(EnhancedStatusCode, &str)> {
    let mut fields = line.splitn(3, '.');
    let class = fields.next()?.parse::<u8>().ok()?;
    if !matches!(class, 2 | 4 | 5) {
        // No other classes are defined
        return None;
    }
    let subject = fields.next()?.parse::<u16>().ok()?;

    let remainder = fields.next()?;
    let mut fields = remainder.splitn(2, ' ');
    let detail = fields.next()?.parse::<u16>().ok()?;
    let remainder = fields.next()?;

    Some((
        EnhancedStatusCode {
            class,
            subject,
            detail,
        },
        remainder,
    ))
}

fn remove_line_break(data: &str) -> String {
    let data = data.as_bytes();
    let mut normalized = Vec::with_capacity(data.len());
    let mut last_idx = 0;

    for i in memchr::memchr2_iter(b'\r', b'\n', data) {
        match data[i] {
            b'\r' => {
                normalized.extend_from_slice(&data[last_idx..i]);
                if data.get(i + 1).copied() != Some(b'\n') {
                    normalized.push(b' ');
                }
            }
            b'\n' => {
                normalized.extend_from_slice(&data[last_idx..i]);
                normalized.push(b' ');
            }
            _ => unreachable!(),
        }
        last_idx = i + 1;
    }

    normalized.extend_from_slice(&data[last_idx..]);
    // This is safe because data comes from str, which is
    // guaranteed to be valid utf8, and all we're manipulating
    // above is whitespace which won't invalidate the utf8
    // byte sequences in the data byte array
    unsafe { String::from_utf8_unchecked(normalized) }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResponseLine<'a> {
    pub code: u16,
    pub is_final: bool,
    pub content: &'a str,
}

impl<'a> ResponseLine<'a> {
    /// Reconsitute the original line that we parsed
    fn to_original_line(&self) -> String {
        format!(
            "{}{}{}",
            self.code,
            if self.is_final { " " } else { "-" },
            self.content
        )
    }
}

pub(crate) struct ResponseBuilder {
    pub code: u16,
    pub enhanced_code: Option<EnhancedStatusCode>,
    pub content: String,
}

impl ResponseBuilder {
    pub fn new(parsed: &ResponseLine) -> Self {
        let code = parsed.code;
        let (enhanced_code, content) = match parse_enhanced_status_code(parsed.content) {
            Some((enhanced, content)) => (Some(enhanced), content.to_string()),
            None => (None, parsed.content.to_string()),
        };

        Self {
            code,
            enhanced_code,
            content,
        }
    }

    pub fn add_line(&mut self, parsed: &ResponseLine) -> Result<(), String> {
        if parsed.code != self.code {
            return Err(parsed.to_original_line());
        }

        self.content.push('\n');

        let mut content = parsed.content;

        if let Some(enh) = &self.enhanced_code {
            let prefix = format!("{}.{}.{} ", enh.class, enh.subject, enh.detail);
            if let Some(remainder) = parsed.content.strip_prefix(&prefix) {
                content = remainder;
            }
        }

        self.content.push_str(content);
        Ok(())
    }

    pub fn build(self, command: Option<String>) -> Response {
        Response {
            code: self.code,
            content: self.content,
            enhanced_code: self.enhanced_code,
            command,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn remove_crlf() {
        fn remove(s: &str, expect: &str) {
            assert_eq!(remove_line_break(s), expect, "input: {s:?}");
        }

        remove("hello\r\nthere\r\n", "hello there ");
        remove("hello\r", "hello ");
        remove("hello\nthere\r\n", "hello there ");
        remove("hello\r\nthere\n", "hello there ");
        remove("hello\r\r\r\nthere\n", "hello   there ");
    }

    #[test]
    fn response_parsing() {
        assert_eq!(
            parse_enhanced_status_code("2.0.1 w00t"),
            Some((
                EnhancedStatusCode {
                    class: 2,
                    subject: 0,
                    detail: 1
                },
                "w00t"
            ))
        );

        assert_eq!(parse_enhanced_status_code("3.0.0 w00t"), None);

        assert_eq!(parse_enhanced_status_code("2.0.0.1 w00t"), None);

        assert_eq!(parse_enhanced_status_code("2.0.0.1w00t"), None);
    }
}

fn as_single_line<S>(content: &String, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&remove_line_break(content))
}
