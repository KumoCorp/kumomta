use ordermap::OrderMap;
use regex::{RegexSet, RegexSetBuilder};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Hash, Clone, Ord, PartialOrd)]
#[serde(from = "String", into = "String")]
pub enum BounceClass {
    PreDefined(PreDefinedBounceClass),
    UserDefined(String),
}

impl From<String> for BounceClass {
    fn from(s: String) -> BounceClass {
        if let Ok(pre) = PreDefinedBounceClass::from_str(&s) {
            BounceClass::PreDefined(pre)
        } else {
            BounceClass::UserDefined(s)
        }
    }
}

impl Into<String> for BounceClass {
    fn into(self) -> String {
        match self {
            BounceClass::PreDefined(pre) => pre.to_string(),
            BounceClass::UserDefined(s) => s,
        }
    }
}

impl Default for BounceClass {
    fn default() -> Self {
        PreDefinedBounceClass::Uncategorized.into()
    }
}

impl From<PreDefinedBounceClass> for BounceClass {
    fn from(c: PreDefinedBounceClass) -> BounceClass {
        BounceClass::PreDefined(c)
    }
}

#[derive(
    Serialize,
    Deserialize,
    Debug,
    PartialEq,
    Eq,
    Hash,
    Copy,
    Clone,
    Ord,
    PartialOrd,
    strum::EnumString,
    strum::Display,
)]
pub enum PreDefinedBounceClass {
    /// The recipient is invalid
    InvalidRecipient,
    /// The message bounced due to a DNS failure.
    DNSFailure,
    /// The message was blocked by the receiver as coming from a known spam source.
    SpamBlock,
    /// The message was blocked by the receiver as spam
    SpamContent,
    /// The message was blocked by the receiver because it contained an attachment
    ProhibitedAttachment,
    /// The message was blocked by the receiver because relaying is not allowed.
    RelayDenied,
    /// The message is an auto-reply/vacation mail.
    AutoReply,
    /// Message transmission has been temporarily delayed.
    TransientFailure,
    /// The message is a subscribe request.
    Subscribe,
    /// The message is an unsubscribe request.
    Unsubscribe,
    /// The message is a challenge-response probe.
    ChallengeResponse,
    /// messages rejected due to configuration issues with remote host, 5.X.X error
    BadConfiguration,
    /// messages bounced due to bad connection issues with remote host, 4.X.X error
    BadConnection,
    /// messages bounced due to invalid or non-existing domains, 5.X.X error
    BadDomain,
    /// messages refused or blocked due to content related reasons, 5.X.X error
    ContentRelated,
    /// messages rejected due to expired, inactive, or disabled recipient addresses, 5.X.X error
    InactiveMailbox,
    /// messages bounced due to invalid DNS or MX entry for sending domain
    InvalidSender,
    /// messages bounced due to not being delivered before the bounce-after, 4.X.X error
    MessageExpired,
    /// messages bounces due to receiving no response from remote host after connecting, 4.X.X or 5.X.X error
    NoAnswerFromHost,
    /// messages refused or blocked due to general policy reasons, 5.X.X error
    PolicyRelated,
    /// messages rejected due to SMTP protocol syntax or sequence errors, 5.X.X error
    ProtocolErrors,
    /// messages rejected or blocked due to mailbox quota issues, 4.X.X or 5.X.X error
    QuotaIssues,
    /// messages refused or blocked due to remote mail server relaying issues, 5.X.X error
    RelayingIssues,
    /// messages bounced due to mail routing issues for recipient domain, 5.X.X error
    RoutingErrors,
    /// messages refused or blocked due to spam related reasons, 5.X.X error
    SpamRelated,
    /// messages refused or blocked due to virus related reasons, 5.X.X error
    VirusRelated,
    /// authentication policy was not met
    AuthenticationFailed,
    /// messages rejected due to other reasons, 4.X.X or 5.X.X error
    Uncategorized,
}

/// Defines the content of bounce classifier rules files
#[derive(Deserialize, Serialize, Debug)]
pub struct BounceClassifierFile {
    pub rules: OrderMap<BounceClass, Vec<String>>,
}

/// Holds state for compiling rules files into a classifier
#[derive(Default)]
pub struct BounceClassifierBuilder {
    rules: Vec<(BounceClass, String)>,
}

impl BounceClassifierBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_rule(&mut self, class: BounceClass, rule: String) {
        self.rules.push((class, rule));
    }

    pub fn merge(&mut self, decoded_file: BounceClassifierFile) {
        for (class, rules) in decoded_file.rules {
            for rule in rules {
                self.add_rule(class.clone(), rule);
            }
        }
    }

    pub fn merge_json_file(&mut self, file_name: &str) -> Result<(), String> {
        let mut f = std::fs::File::open(file_name)
            .map_err(|err| format!("reading file: {file_name}: {err:#}"))?;
        let decoded: BounceClassifierFile = serde_json::from_reader(&mut f)
            .map_err(|err| format!("decoding {file_name} as BounceClassifierFile: {err:#}"))?;
        self.merge(decoded);
        Ok(())
    }

    pub fn merge_toml_file(&mut self, file_name: &str) -> Result<(), String> {
        let data = std::fs::read_to_string(file_name)
            .map_err(|err| format!("reading file: {file_name}: {err:#}"))?;
        let decoded: BounceClassifierFile = toml::from_str(&data)
            .map_err(|err| format!("decoding {file_name} as BounceClassifierFile: {err:#}"))?;
        self.merge(decoded);
        Ok(())
    }

    pub fn build(self) -> Result<BounceClassifier, String> {
        let mut pattern_to_class = vec![];
        let mut patterns = vec![];
        for (class, rule) in self.rules {
            // Build a simple implicit reverse map from pattern
            // index to the bounce classification. This gives
            // an O(1) mapping from the regex result at the
            // cost of O(n) memory. If the rules get very large,
            // this could be changed to a structure that tracks
            // start/end ranges of pattern indices and uses a
            // binary search.
            pattern_to_class.push(class.clone());
            patterns.push(rule);
        }

        pattern_to_class.shrink_to_fit();

        let set = RegexSetBuilder::new(patterns)
            .build()
            .map_err(|err| format!("compiling rules: {err:#}"))?;
        Ok(BounceClassifier {
            set,
            pattern_to_class,
        })
    }
}

pub struct BounceClassifier {
    set: RegexSet,
    pattern_to_class: Vec<BounceClass>,
}

impl BounceClassifier {
    pub fn classify_str(&self, s: &str) -> BounceClass {
        self.set
            .matches(s)
            .into_iter()
            .next()
            .and_then(|idx| self.pattern_to_class.get(idx))
            .cloned()
            .unwrap_or(BounceClass::PreDefined(
                PreDefinedBounceClass::Uncategorized,
            ))
    }

    pub fn classify_response(&self, response: &rfc5321::Response) -> BounceClass {
        let line = response.to_single_line();
        self.classify_str(&line)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_rule_order() {
        let f1: BounceClassifierFile = toml::from_str(
            r#"
[rules]
foo = ["woot", "aaa"]
bar = ["woot", "aaa", "bbb"]
        "#,
        )
        .unwrap();

        let f2: BounceClassifierFile = toml::from_str(
            r#"
[rules]
second_file = ["bbb", "ccc"]
        "#,
        )
        .unwrap();

        let mut builder = BounceClassifierBuilder::new();
        builder.merge(f1);
        builder.merge(f2);

        let classifier = builder.build().unwrap();
        assert_eq!(
            classifier.classify_str("woot"),
            BounceClass::UserDefined("foo".to_string()),
            "foo should match rather than bar"
        );
        assert_eq!(
            classifier.classify_str("aaa"),
            BounceClass::UserDefined("foo".to_string()),
            "foo should match rather than bar"
        );
        assert_eq!(
            classifier.classify_str("bbb"),
            BounceClass::UserDefined("bar".to_string()),
        );
        assert_eq!(
            classifier.classify_str("ccc"),
            BounceClass::UserDefined("second_file".to_string()),
        );
    }

    #[test]
    fn test_bounce_classify_iana() {
        let mut builder = BounceClassifierBuilder::new();
        builder
            .merge_toml_file("../../assets/bounce_classifier/iana.toml")
            .unwrap();
        let classifier = builder.build().unwrap();

        let corpus = &[
            (
                "552 5.2.2 mailbox is stuffed",
                PreDefinedBounceClass::QuotaIssues,
            ),
            (
                "552 4.2.2 mailbox is stuffed",
                PreDefinedBounceClass::QuotaIssues,
            ),
            (
                "552 4.2.2 mailbox is stuffed",
                PreDefinedBounceClass::QuotaIssues,
            ),
            (
                "352 5.2.2 mailbox is stuffed",
                PreDefinedBounceClass::Uncategorized,
            ),
            (
                "525 4.7.13 user account is disabled",
                PreDefinedBounceClass::InactiveMailbox,
            ),
            (
                "551 4.7.17 mailbox owner has changed",
                PreDefinedBounceClass::InvalidRecipient,
            ),
            (
                "551 4.7.18 domain owner has changed",
                PreDefinedBounceClass::BadDomain,
            ),
        ];

        for &(input, output) in corpus {
            assert_eq!(
                classifier.classify_str(input),
                output.into(),
                "expected {input} -> {output:?}"
            );
        }
    }
}
