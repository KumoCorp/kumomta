use crate::client::SmtpClientTimeouts;
use pest::iterators::{Pair, Pairs};
use pest::Parser as _;
use pest_derive::*;
use std::time::Duration;

#[derive(Parser)]
#[grammar = "rfc5321.pest"]
struct Parser;

impl Parser {
    pub fn parse_command(text: &str) -> Result<Command, String> {
        let result = Parser::parse(Rule::command, text)
            .map_err(|err| format!("{err:#}"))?
            .next()
            .unwrap();

        match result.as_rule() {
            Rule::mail => Self::parse_mail(result.into_inner()),
            Rule::rcpt => Self::parse_rcpt(result.into_inner()),
            Rule::ehlo => Self::parse_ehlo(result.into_inner()),
            Rule::helo => Self::parse_helo(result.into_inner()),
            Rule::data => Ok(Command::Data),
            Rule::rset => Ok(Command::Rset),
            Rule::quit => Ok(Command::Quit),
            Rule::starttls => Ok(Command::StartTls),
            Rule::vrfy => Self::parse_vrfy(result.into_inner()),
            Rule::expn => Self::parse_expn(result.into_inner()),
            Rule::help => Self::parse_help(result.into_inner()),
            Rule::noop => Self::parse_noop(result.into_inner()),
            Rule::auth => Self::parse_auth(result.into_inner()),
            _ => Err(format!("unexpected {result:?}")),
        }
    }

    fn parse_ehlo(mut pairs: Pairs<Rule>) -> Result<Command, String> {
        let domain = pairs.next().unwrap();
        Ok(Command::Ehlo(Self::parse_domain(domain)?))
    }

    fn parse_helo(mut pairs: Pairs<Rule>) -> Result<Command, String> {
        let domain = pairs.next().unwrap();
        Ok(Command::Helo(Self::parse_domain(domain)?))
    }

    fn parse_vrfy(mut pairs: Pairs<Rule>) -> Result<Command, String> {
        let param = pairs.next().unwrap().as_str().to_string();
        Ok(Command::Vrfy(param))
    }

    fn parse_expn(mut pairs: Pairs<Rule>) -> Result<Command, String> {
        let param = pairs.next().unwrap().as_str().to_string();
        Ok(Command::Expn(param))
    }

    fn parse_help(mut pairs: Pairs<Rule>) -> Result<Command, String> {
        let param = pairs.next().map(|s| s.as_str().to_string());
        Ok(Command::Help(param))
    }

    fn parse_noop(mut pairs: Pairs<Rule>) -> Result<Command, String> {
        let param = pairs.next().map(|s| s.as_str().to_string());
        Ok(Command::Noop(param))
    }

    fn parse_auth(mut pairs: Pairs<Rule>) -> Result<Command, String> {
        let sasl_mech = pairs.next().map(|s| s.as_str().to_string()).unwrap();
        let initial_response = pairs.next().map(|s| s.as_str().to_string());

        Ok(Command::Auth {
            sasl_mech,
            initial_response,
        })
    }

    fn parse_rcpt(mut pairs: Pairs<Rule>) -> Result<Command, String> {
        let forward_path = pairs.next().unwrap().into_inner().next().unwrap();
        let mut no_angles = false;
        let address = match forward_path.as_rule() {
            Rule::path_no_angles => {
                no_angles = true;
                ForwardPath::Path(Self::parse_path(forward_path)?)
            }
            Rule::path => ForwardPath::Path(Self::parse_path(forward_path)?),
            Rule::postmaster => ForwardPath::Postmaster,
            wat => return Err(format!("unexpected {wat:?}")),
        };

        let mut parameters = vec![];

        if let Some(params) = pairs.next() {
            if no_angles {
                return Err(format!(
                    "must enclose address in <> if you want to use ESMTP parameters"
                ));
            }
            for param in params.into_inner() {
                let mut iter = param.into_inner();
                let name = iter.next().unwrap().as_str().to_string();
                let value = iter.next().map(|p| p.as_str().to_string());
                parameters.push(EsmtpParameter { name, value });
            }
        }

        Ok(Command::RcptTo {
            address,
            parameters,
        })
    }

    fn parse_mail(mut pairs: Pairs<Rule>) -> Result<Command, String> {
        let reverse_path = pairs.next().unwrap().into_inner().next().unwrap();
        let mut no_angles = false;
        let address = match reverse_path.as_rule() {
            Rule::path_no_angles => {
                no_angles = true;
                ReversePath::Path(Self::parse_path(reverse_path)?)
            }
            Rule::path => ReversePath::Path(Self::parse_path(reverse_path)?),
            Rule::null_sender => ReversePath::NullSender,
            wat => return Err(format!("unexpected {wat:?}")),
        };

        let mut parameters = vec![];

        if let Some(params) = pairs.next() {
            if no_angles {
                return Err(format!(
                    "must enclose address in <> if you want to use ESMTP parameters"
                ));
            }
            for param in params.into_inner() {
                let mut iter = param.into_inner();
                let name = iter.next().unwrap().as_str().to_string();
                let value = iter.next().map(|p| p.as_str().to_string());
                parameters.push(EsmtpParameter { name, value });
            }
        }

        Ok(Command::MailFrom {
            address,
            parameters,
        })
    }

    fn parse_path(path: Pair<Rule>) -> Result<MailPath, String> {
        let mut at_domain_list: Vec<String> = vec![];
        for p in path.into_inner() {
            match p.as_rule() {
                Rule::adl => {
                    for pair in p.into_inner() {
                        if let Some(dom) = pair.into_inner().next() {
                            at_domain_list.push(dom.as_str().to_string());
                        }
                    }
                }
                Rule::mailbox => {
                    let mailbox = Self::parse_mailbox(p.into_inner())?;
                    return Ok(MailPath {
                        at_domain_list,
                        mailbox,
                    });
                }
                _ => unreachable!(),
            }
        }
        unreachable!()
    }

    fn parse_domain(domain: Pair<Rule>) -> Result<Domain, String> {
        Ok(match domain.as_rule() {
            Rule::domain => Domain::Name(domain.as_str().to_string()),
            Rule::address_literal => {
                let literal = domain.into_inner().next().unwrap();
                match literal.as_rule() {
                    Rule::ipv4_address_literal => Domain::V4(literal.as_str().to_string()),
                    Rule::ipv6_address_literal => {
                        Domain::V6(literal.into_inner().next().unwrap().as_str().to_string())
                    }
                    Rule::general_address_literal => {
                        let mut literal = literal.into_inner();
                        let tag = literal.next().unwrap().as_str().to_string();
                        let literal = literal.next().unwrap().as_str().to_string();
                        Domain::Tagged { tag, literal }
                    }

                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        })
    }

    fn parse_mailbox(mut mailbox: Pairs<Rule>) -> Result<Mailbox, String> {
        let local_part = mailbox.next().unwrap().as_str().to_string();
        let domain = Self::parse_domain(mailbox.next().unwrap())?;
        Ok(Mailbox { local_part, domain })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReversePath {
    Path(MailPath),
    NullSender,
}

impl TryFrom<&str> for ReversePath {
    type Error = &'static str;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        if s.is_empty() {
            Ok(Self::NullSender)
        } else {
            let fields: Vec<&str> = s.split('@').collect();
            if fields.len() == 2 {
                Ok(Self::Path(MailPath {
                    at_domain_list: vec![],
                    mailbox: Mailbox {
                        local_part: fields[0].to_string(),
                        domain: Domain::Name(fields[1].to_string()),
                    },
                }))
            } else {
                Err("wrong number of @ signs")
            }
        }
    }
}

impl ToString for ReversePath {
    fn to_string(&self) -> String {
        match self {
            Self::Path(p) => p.to_string(),
            Self::NullSender => "".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardPath {
    Path(MailPath),
    Postmaster,
}

impl TryFrom<&str> for ForwardPath {
    type Error = &'static str;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        if s.is_empty() {
            Err("cannot send to null sender")
        } else if s.eq_ignore_ascii_case("postmaster") {
            Ok(Self::Postmaster)
        } else {
            let fields: Vec<&str> = s.split('@').collect();
            if fields.len() == 2 {
                Ok(Self::Path(MailPath {
                    at_domain_list: vec![],
                    mailbox: Mailbox {
                        local_part: fields[0].to_string(),
                        domain: Domain::Name(fields[1].to_string()),
                    },
                }))
            } else {
                Err("wrong number of @ signs")
            }
        }
    }
}

impl ToString for ForwardPath {
    fn to_string(&self) -> String {
        match self {
            Self::Path(p) => p.to_string(),
            Self::Postmaster => "postmaster".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailPath {
    pub at_domain_list: Vec<String>,
    pub mailbox: Mailbox,
}

impl ToString for MailPath {
    fn to_string(&self) -> String {
        // Note: RFC5321 says about at_domain_list:
        // Note that this form, the so-called "source
        // route", MUST BE accepted, SHOULD NOT be
        // generated, and SHOULD be ignored.
        // So we don't include it in the stringified
        // version of MailPath
        self.mailbox.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mailbox {
    pub local_part: String,
    pub domain: Domain,
}

impl ToString for Mailbox {
    fn to_string(&self) -> String {
        let domain = self.domain.to_string();
        format!("{}@{}", self.local_part, domain)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Domain {
    Name(String),
    V4(String),
    V6(String),
    Tagged { tag: String, literal: String },
}

impl ToString for Domain {
    fn to_string(&self) -> String {
        match self {
            Self::Name(name) => name.to_string(),
            Self::V4(addr) => format!("[{addr}]"),
            Self::V6(addr) => format!("[IPv6:{addr}]"),
            Self::Tagged { tag, literal } => format!("[{tag}:{literal}]"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EsmtpParameter {
    pub name: String,
    pub value: Option<String>,
}

impl ToString for EsmtpParameter {
    fn to_string(&self) -> String {
        match &self.value {
            Some(value) => format!("{}={}", self.name, value),
            None => self.name.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Ehlo(Domain),
    Helo(Domain),
    MailFrom {
        address: ReversePath,
        parameters: Vec<EsmtpParameter>,
    },
    RcptTo {
        address: ForwardPath,
        parameters: Vec<EsmtpParameter>,
    },
    Data,
    DataDot,
    Rset,
    Quit,
    Vrfy(String),
    Expn(String),
    Help(Option<String>),
    Noop(Option<String>),
    StartTls,
    Auth {
        sasl_mech: String,
        initial_response: Option<String>,
    },
}

impl Command {
    pub fn parse(line: &str) -> Result<Self, String> {
        Parser::parse_command(line)
    }

    pub fn encode(&self) -> String {
        match self {
            Self::Ehlo(domain) => format!("EHLO {}\r\n", domain.to_string()),
            Self::Helo(domain) => format!("HELO {}\r\n", domain.to_string()),
            Self::MailFrom {
                address,
                parameters,
            } => {
                let mut params = String::new();
                for p in parameters {
                    params.push(' ');
                    params.push_str(&p.to_string());
                }

                format!("MAIL FROM:<{}>{params}\r\n", address.to_string())
            }
            Self::RcptTo {
                address,
                parameters,
            } => {
                let mut params = String::new();
                for p in parameters {
                    params.push(' ');
                    params.push_str(&p.to_string());
                }

                format!("RCPT TO:<{}>{params}\r\n", address.to_string())
            }
            Self::Data => "DATA\r\n".to_string(),
            Self::DataDot => ".\r\n".to_string(),
            Self::Rset => "RSET\r\n".to_string(),
            Self::Quit => "QUIT\r\n".to_string(),
            Self::StartTls => "STARTTLS\r\n".to_string(),
            Self::Vrfy(param) => format!("VRFY {param}\r\n"),
            Self::Expn(param) => format!("EXPN {param}\r\n"),
            Self::Help(Some(param)) => format!("HELP {param}\r\n"),
            Self::Help(None) => format!("HELP\r\n"),
            Self::Noop(Some(param)) => format!("NOOP {param}\r\n"),
            Self::Noop(None) => format!("NOOP\r\n"),
            Self::Auth {
                sasl_mech,
                initial_response: None,
            } => format!("AUTH {sasl_mech}\r\n"),
            Self::Auth {
                sasl_mech,
                initial_response: Some(resp),
            } => format!("AUTH {sasl_mech} {resp}\r\n"),
        }
    }

    /// Timeouts for reading the response
    pub fn client_timeout(&self, timeouts: &SmtpClientTimeouts) -> Duration {
        match self {
            Self::Helo(_) | Self::Ehlo(_) => timeouts.ehlo_timeout,
            Self::MailFrom { .. } => timeouts.mail_from_timeout,
            Self::RcptTo { .. } => timeouts.rcpt_to_timeout,
            Self::Data { .. } => timeouts.data_timeout,
            Self::DataDot => timeouts.data_dot_timeout,
            Self::Rset => timeouts.rset_timeout,
            Self::StartTls => timeouts.starttls_timeout,
            Self::Quit | Self::Vrfy(_) | Self::Expn(_) | Self::Help(_) | Self::Noop(_) => {
                timeouts.idle_timeout
            }
            Self::Auth { .. } => timeouts.auth_timeout,
        }
    }

    /// Timeouts for writing the request
    pub fn client_timeout_request(&self, timeouts: &SmtpClientTimeouts) -> Duration {
        let one_minute = Duration::from_secs(60);
        self.client_timeout(timeouts).min(one_minute)
    }
}

pub fn is_valid_domain(text: &str) -> bool {
    Parser::parse(Rule::complete_domain, text).is_ok()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse_single_verbs() {
        assert_eq!(Parser::parse_command("data").unwrap(), Command::Data,);
        assert_eq!(Parser::parse_command("Quit").unwrap(), Command::Quit,);
        assert_eq!(Parser::parse_command("rset").unwrap(), Command::Rset,);
    }

    #[test]
    fn parse_vrfy() {
        assert_eq!(
            Parser::parse_command("VRFY someone").unwrap(),
            Command::Vrfy("someone".to_string())
        );
    }

    #[test]
    fn parse_expn() {
        assert_eq!(
            Parser::parse_command("expn someone").unwrap(),
            Command::Expn("someone".to_string())
        );
    }

    #[test]
    fn parse_help() {
        assert_eq!(Parser::parse_command("help").unwrap(), Command::Help(None),);
        assert_eq!(
            Parser::parse_command("help me").unwrap(),
            Command::Help(Some("me".to_string())),
        );
    }

    #[test]
    fn parse_noop() {
        assert_eq!(Parser::parse_command("noop").unwrap(), Command::Noop(None),);
        assert_eq!(
            Parser::parse_command("noop param").unwrap(),
            Command::Noop(Some("param".to_string())),
        );
    }

    #[test]
    fn parse_ehlo() {
        assert_eq!(
            Parser::parse_command("EHLO there").unwrap(),
            Command::Ehlo(Domain::Name("there".to_string()))
        );
        assert_eq!(
            Parser::parse_command("EHLO [127.0.0.1]").unwrap(),
            Command::Ehlo(Domain::V4("127.0.0.1".to_string()))
        );
    }

    #[test]
    fn parse_helo() {
        assert_eq!(
            Parser::parse_command("HELO there").unwrap(),
            Command::Helo(Domain::Name("there".to_string()))
        );
        // The spec says that we cannot use address literals with,
        // HELO, but some tools will still submit it and some MTAs
        // will accept it, so we do too.
        assert_eq!(
            Parser::parse_command("EHLO [127.0.0.1]").unwrap(),
            Command::Ehlo(Domain::V4("127.0.0.1".to_string()))
        );
    }

    #[test]
    fn parse_auth() {
        assert_eq!(
            Parser::parse_command("AUTH PLAIN dGVzdAB0ZXN0ADEyMzQ=").unwrap(),
            Command::Auth {
                sasl_mech: "PLAIN".to_string(),
                initial_response: Some("dGVzdAB0ZXN0ADEyMzQ=".to_string()),
            }
        );
        assert_eq!(
            Parser::parse_command("AUTH PLAIN").unwrap(),
            Command::Auth {
                sasl_mech: "PLAIN".to_string(),
                initial_response: None,
            }
        );
    }

    #[test]
    fn parse_rcpt_to() {
        assert_eq!(
            Parser::parse_command("Rcpt To:<user@host>").unwrap(),
            Command::RcptTo {
                address: ForwardPath::Path(MailPath {
                    at_domain_list: vec![],
                    mailbox: Mailbox {
                        local_part: "user".to_string(),
                        domain: Domain::Name("host".to_string())
                    }
                }),
                parameters: vec![],
            }
        );
        assert_eq!(
            Parser::parse_command("Rcpt To:user@host").unwrap(),
            Command::RcptTo {
                address: ForwardPath::Path(MailPath {
                    at_domain_list: vec![],
                    mailbox: Mailbox {
                        local_part: "user".to_string(),
                        domain: Domain::Name("host".to_string())
                    }
                }),
                parameters: vec![],
            }
        );

        assert_eq!(
            Parser::parse_command("Rcpt To:  user@host").unwrap(),
            Command::RcptTo {
                address: ForwardPath::Path(MailPath {
                    at_domain_list: vec![],
                    mailbox: Mailbox {
                        local_part: "user".to_string(),
                        domain: Domain::Name("host".to_string())
                    }
                }),
                parameters: vec![],
            }
        );

        assert_eq!(
            Parser::parse_command("Rcpt To:<admin@[2001:aaaa:bbbbb]>").unwrap(),
            Command::RcptTo {
                address: ForwardPath::Path(MailPath {
                    at_domain_list: vec![],
                    mailbox: Mailbox {
                        local_part: "admin".to_string(),
                        domain: Domain::Tagged {
                            tag: "2001".to_string(),
                            literal: "aaaa:bbbbb".to_string()
                        }
                    }
                }),
                parameters: vec![],
            }
        );

        assert_eq!(
            Domain::Tagged {
                tag: "2001".to_string(),
                literal: "aaaa:bbbbb".to_string()
            }
            .to_string(),
            "[2001:aaaa:bbbbb]".to_string()
        );

        assert_eq!(
            Parser::parse_command("Rcpt To:<\"asking for trouble\"@host.name>").unwrap(),
            Command::RcptTo {
                address: ForwardPath::Path(MailPath {
                    at_domain_list: vec![],
                    mailbox: Mailbox {
                        local_part: "\"asking for trouble\"".to_string(),
                        domain: Domain::Name("host.name".to_string())
                    }
                }),
                parameters: vec![],
            }
        );

        assert_eq!(
            Parser::parse_command("Rcpt To:<PostMastER>").unwrap(),
            Command::RcptTo {
                address: ForwardPath::Postmaster,
                parameters: vec![],
            }
        );

        assert_eq!(
            Parser::parse_command("Rcpt To:<user@host> woot").unwrap(),
            Command::RcptTo {
                address: ForwardPath::Path(MailPath {
                    at_domain_list: vec![],
                    mailbox: Mailbox {
                        local_part: "user".to_string(),
                        domain: Domain::Name("host".to_string())
                    }
                }),
                parameters: vec![EsmtpParameter {
                    name: "woot".to_string(),
                    value: None
                }],
            }
        );

        assert_eq!(
            Parser::parse_command("Rcpt To:user@host woot").unwrap_err(),
            "must enclose address in <> if you want to use ESMTP parameters".to_string()
        );
    }

    #[test]
    fn parse_mail_from() {
        assert_eq!(
            Parser::parse_command("Mail FROM:<user@host>").unwrap(),
            Command::MailFrom {
                address: ReversePath::Path(MailPath {
                    at_domain_list: vec![],
                    mailbox: Mailbox {
                        local_part: "user".to_string(),
                        domain: Domain::Name("host".to_string())
                    }
                }),
                parameters: vec![],
            }
        );

        assert_eq!(
            Parser::parse_command("Mail FROM:user@host").unwrap(),
            Command::MailFrom {
                address: ReversePath::Path(MailPath {
                    at_domain_list: vec![],
                    mailbox: Mailbox {
                        local_part: "user".to_string(),
                        domain: Domain::Name("host".to_string())
                    }
                }),
                parameters: vec![],
            }
        );

        assert_eq!(
            Parser::parse_command("Mail FROM:user@host foo bar=baz").unwrap_err(),
            "must enclose address in <> if you want to use ESMTP parameters".to_string()
        );

        assert_eq!(
            Parser::parse_command("Mail FROM:<user@host> foo bar=baz").unwrap(),
            Command::MailFrom {
                address: ReversePath::Path(MailPath {
                    at_domain_list: vec![],
                    mailbox: Mailbox {
                        local_part: "user".to_string(),
                        domain: Domain::Name("host".to_string())
                    }
                }),
                parameters: vec![
                    EsmtpParameter {
                        name: "foo".to_string(),
                        value: None,
                    },
                    EsmtpParameter {
                        name: "bar".to_string(),
                        value: Some("baz".to_string()),
                    }
                ],
            }
        );

        assert_eq!(
            Parser::parse_command("mail from:<user@[10.0.0.1]>").unwrap(),
            Command::MailFrom {
                address: ReversePath::Path(MailPath {
                    at_domain_list: vec![],
                    mailbox: Mailbox {
                        local_part: "user".to_string(),
                        domain: Domain::V4("10.0.0.1".to_string())
                    }
                }),
                parameters: vec![],
            }
        );

        assert_eq!(
            Parser::parse_command("mail from:<user@[IPv6:::1]>").unwrap(),
            Command::MailFrom {
                address: ReversePath::Path(MailPath {
                    at_domain_list: vec![],
                    mailbox: Mailbox {
                        local_part: "user".to_string(),
                        domain: Domain::V6("::1".to_string())
                    }
                }),
                parameters: vec![],
            }
        );

        assert_eq!(
            Mailbox {
                local_part: "user".to_string(),
                domain: Domain::V6("::1".to_string())
            }
            .to_string(),
            "user@[IPv6:::1]".to_string()
        );

        assert_eq!(
            Parser::parse_command("mail from:<user@[future:something]>").unwrap(),
            Command::MailFrom {
                address: ReversePath::Path(MailPath {
                    at_domain_list: vec![],
                    mailbox: Mailbox {
                        local_part: "user".to_string(),
                        domain: Domain::Tagged {
                            tag: "future".to_string(),
                            literal: "something".to_string()
                        }
                    }
                }),
                parameters: vec![],
            }
        );

        assert_eq!(
            Parser::parse_command("MAIL FROM:<@hosta.int,@jkl.org:userc@d.bar.org>").unwrap(),
            Command::MailFrom {
                address: ReversePath::Path(MailPath {
                    at_domain_list: vec!["hosta.int".to_string(), "jkl.org".to_string()],
                    mailbox: Mailbox {
                        local_part: "userc".to_string(),
                        domain: Domain::Name("d.bar.org".to_string())
                    }
                }),
                parameters: vec![],
            }
        );
    }

    #[test]
    fn parse_domain() {
        assert!(is_valid_domain("hello"));
        assert!(is_valid_domain("he-llo"));
        assert!(is_valid_domain("he.llo"));
        assert!(is_valid_domain("he.llo-"));
    }
}

/* ABNF from RFC 5321

mail = "MAIL FROM:" Reverse-path [SP Mail-parameters] CRLF

rcpt = "RCPT TO:" ( "<Postmaster@" Domain ">" / "<Postmaster>" /
                Forward-path ) [SP Rcpt-parameters] CRLF

                Note that, in a departure from the usual rules for
                local-parts, the "Postmaster" string shown above is
                treated as case-insensitive.

Reverse-path   = Path / "<>"
Forward-path   = Path
Path           = "<" [ A-d-l ":" ] Mailbox ">"
A-d-l          = At-domain *( "," At-domain )
                  ; Note that this form, the so-called "source
                  ; route", MUST BE accepted, SHOULD NOT be
                  ; generated, and SHOULD be ignored.
At-domain      = "@" Domain
Mail-parameters  = esmtp-param *(SP esmtp-param)

   Rcpt-parameters  = esmtp-param *(SP esmtp-param)

   esmtp-param    = esmtp-keyword ["=" esmtp-value]

   esmtp-keyword  = (ALPHA / DIGIT) *(ALPHA / DIGIT / "-")

   esmtp-value    = 1*(%d33-60 / %d62-126)
                  ; any CHAR excluding "=", SP, and control
                  ; characters.  If this string is an email address,
                  ; i.e., a Mailbox, then the "xtext" syntax [32]
                  ; SHOULD be used.

   Keyword        = Ldh-str

   Argument       = Atom

   Domain         = sub-domain *("." sub-domain)
   sub-domain     = Let-dig [Ldh-str]

   Let-dig        = ALPHA / DIGIT

   Ldh-str        = *( ALPHA / DIGIT / "-" ) Let-dig

   address-literal  = "[" ( IPv4-address-literal /
                    IPv6-address-literal /
                    General-address-literal ) "]"
                    ; See Section 4.1.3

   Mailbox        = Local-part "@" ( Domain / address-literal )

   Local-part     = Dot-string / Quoted-string
                  ; MAY be case-sensitive


   Dot-string     = Atom *("."  Atom)

   Atom           = 1*atext

   Quoted-string  = DQUOTE *QcontentSMTP DQUOTE

   QcontentSMTP   = qtextSMTP / quoted-pairSMTP

   quoted-pairSMTP  = %d92 %d32-126
                    ; i.e., backslash followed by any ASCII
                    ; graphic (including itself) or SPace

   qtextSMTP      = %d32-33 / %d35-91 / %d93-126
                  ; i.e., within a quoted string, any
                  ; ASCII graphic or space is permitted
                  ; without blackslash-quoting except
                  ; double-quote and the backslash itself.

   String         = Atom / Quoted-string


      IPv4-address-literal  = Snum 3("."  Snum)

   IPv6-address-literal  = "IPv6:" IPv6-addr

   General-address-literal  = Standardized-tag ":" 1*dcontent

   Standardized-tag  = Ldh-str
                     ; Standardized-tag MUST be specified in a
                     ; Standards-Track RFC and registered with IANA


   dcontent       = %d33-90 / ; Printable US-ASCII
                  %d94-126 ; excl. "[", "\", "]"

   Snum           = 1*3DIGIT
                  ; representing a decimal integer
                  ; value in the range 0 through 255

   IPv6-addr      = IPv6-full / IPv6-comp / IPv6v4-full / IPv6v4-comp

   IPv6-hex       = 1*4HEXDIG

   IPv6-full      = IPv6-hex 7(":" IPv6-hex)

   IPv6-comp      = [IPv6-hex *5(":" IPv6-hex)] "::"
                  [IPv6-hex *5(":" IPv6-hex)]
                  ; The "::" represents at least 2 16-bit groups of
                  ; zeros.  No more than 6 groups in addition to the
                  ; "::" may be present.

   IPv6v4-full    = IPv6-hex 5(":" IPv6-hex) ":" IPv4-address-literal

   IPv6v4-comp    = [IPv6-hex *3(":" IPv6-hex)] "::"
                  [IPv6-hex *3(":" IPv6-hex) ":"]
                  IPv4-address-literal
                  ; The "::" represents at least 2 16-bit groups of
                  ; zeros.  No more than 4 groups in addition to the
                  ; "::" and IPv4-address-literal may be present.


   ehlo           = "EHLO" SP ( Domain / address-literal ) CRLF
   helo           = "HELO" SP Domain CRLF

   ehlo-ok-rsp    = ( "250" SP Domain [ SP ehlo-greet ] CRLF )
                    / ( "250-" Domain [ SP ehlo-greet ] CRLF
                    *( "250-" ehlo-line CRLF )
                    "250" SP ehlo-line CRLF )

   ehlo-greet     = 1*(%d0-9 / %d11-12 / %d14-127)
                    ; string of any characters other than CR or LF

   ehlo-line      = ehlo-keyword *( SP ehlo-param )

   ehlo-keyword   = (ALPHA / DIGIT) *(ALPHA / DIGIT / "-")
                    ; additional syntax of ehlo-params depends on
                    ; ehlo-keyword

   ehlo-param     = 1*(%d33-126)
                    ; any CHAR excluding <SP> and all
                    ; control characters (US-ASCII 0-31 and 127
                    ; inclusive)

    data = "DATA" CRLF
    rset = "RSET" CRLF
    vrfy = "VRFY" SP String CRLF
    expn = "EXPN" SP String CRLF
    help = "HELP" [ SP String ] CRLF
    noop = "NOOP" [ SP String ] CRLF
quit = "QUIT" CRLF


*/
