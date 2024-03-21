use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
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
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone, Copy, Hash)]
pub struct EnhancedStatusCode {
    pub class: u8,
    pub subject: u16,
    pub detail: u16,
}

fn remove_line_break(line: &String) -> String {
    let mut new_line = String::new();
    let mut cr_to_space = false;

    for c in line.chars() {
        match c {
            '\r' => {
                new_line.push_str(" ");
                cr_to_space = true;
            }
            '\n' => {
                if !cr_to_space {
                    new_line.push_str(" ");
                } else {
                    cr_to_space = false;
                }
            }
            c => new_line.push(c),
        }
    }
    new_line
}

fn as_single_line<S>(content: &String, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&remove_line_break(content))
}
