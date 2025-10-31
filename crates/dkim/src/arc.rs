use crate::header::{ARCMessageSignatureHeader, ARCSealHeader};
use crate::{verify_email_header, DKIMError, ParsedEmail};
use dns_resolver::Resolver;
use mailparsing::{ARCAuthenticationResults, Header};
use std::collections::BTreeMap;
use std::str::FromStr;

pub const MAX_ARC_INSTANCE: u8 = 50;
pub const ARC_MESSAGE_SIGNATURE_HEADER_NAME: &str = "ARC-Message-Signature";
pub const ARC_SEAL_HEADER_NAME: &str = "ARC-Seal";
pub const ARC_AUTHENTICATION_RESULTS_HEADER_NAME: &str = "ARC-Authentication-Results";

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ChainValidationStatus {
    None,
    Fail,
    Pass,
}

impl FromStr for ChainValidationStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<ChainValidationStatus, String> {
        if s.eq_ignore_ascii_case("none") {
            Ok(ChainValidationStatus::None)
        } else if s.eq_ignore_ascii_case("fail") {
            Ok(ChainValidationStatus::Fail)
        } else if s.eq_ignore_ascii_case("pass") {
            Ok(ChainValidationStatus::Pass)
        } else {
            Err(format!("invalid ChainValidationStatus {s}"))
        }
    }
}

#[derive(Debug)]
pub struct ARC {
    pub sets: Vec<ARCSet>,
    /// The instance number of the oldest pass that still validates
    pub last_validated_instance: u8,
    pub issues: Vec<ARCIssue>,
}

#[derive(Debug)]
pub struct ARCIssue {
    pub reason: String,
    pub error: Option<DKIMError>,
    pub header: Option<Header<'static>>,
}

impl ARC {
    pub fn chain_validation_status(&self) -> ChainValidationStatus {
        if self.issues.is_empty() {
            if self.sets.is_empty() {
                ChainValidationStatus::None
            } else if self.last_validated_instance as usize == self.sets.len() {
                ChainValidationStatus::Pass
            } else {
                ChainValidationStatus::Fail
            }
        } else {
            ChainValidationStatus::Fail
        }
    }

    pub async fn verify(email: &ParsedEmail<'_>, resolver: &dyn Resolver) -> Self {
        let mut seals = BTreeMap::new();
        let mut sigs = BTreeMap::new();
        let mut aars = BTreeMap::new();

        let headers = email.get_headers();
        let mut issues = vec![];

        for hdr in headers.iter_named(ARC_SEAL_HEADER_NAME) {
            match ARCSealHeader::parse(hdr.get_raw_value()) {
                Ok(seal) => {
                    let instance = seal.arc_instance().expect("validated by parse");
                    seals
                        .entry(instance)
                        .or_insert_with(Vec::new)
                        .push((seal, hdr.to_owned()));
                }
                Err(err) => {
                    issues.push(ARCIssue {
                        reason: format!("An {ARC_SEAL_HEADER_NAME} header could not be parsed"),
                        error: Some(err),
                        header: Some(hdr.to_owned()),
                    });
                }
            }
        }

        for hdr in headers.iter_named(ARC_MESSAGE_SIGNATURE_HEADER_NAME) {
            match ARCMessageSignatureHeader::parse(hdr.get_raw_value()) {
                Ok(sig) => {
                    let instance = sig.arc_instance().expect("validated by parse");
                    sigs.entry(instance)
                        .or_insert_with(Vec::new)
                        .push((sig, hdr.to_owned()));
                }
                Err(err) => {
                    issues.push(ARCIssue {
                        reason: format!(
                            "An {ARC_MESSAGE_SIGNATURE_HEADER_NAME} header could not be parsed"
                        ),
                        error: Some(err),
                        header: Some(hdr.to_owned()),
                    });
                }
            }
        }

        for hdr in headers.iter_named(ARC_AUTHENTICATION_RESULTS_HEADER_NAME) {
            match hdr.as_arc_authentication_results() {
                Ok(aar) => {
                    if aar.instance == 0 || aar.instance > MAX_ARC_INSTANCE {
                        issues.push(ARCIssue {
                            reason: format!(
                                "An {ARC_AUTHENTICATION_RESULTS_HEADER_NAME} header \
                                    has an invalid instance value"
                            ),
                            error: Some(DKIMError::InvalidARCInstance),
                            header: Some(hdr.to_owned()),
                        });
                        continue;
                    }
                    aars.entry(aar.instance)
                        .or_insert_with(Vec::new)
                        .push((aar, hdr.to_owned()));
                }
                Err(err) => {
                    issues.push(ARCIssue {
                        reason: format!(
                            "An {ARC_AUTHENTICATION_RESULTS_HEADER_NAME} header \
                                    could not be parsed"
                        ),
                        error: Some(err.into()),
                        header: Some(hdr.to_owned()),
                    });
                }
            }
        }

        let mut arc_sets = BTreeMap::new();
        for instance in 1..=MAX_ARC_INSTANCE {
            match (
                seals.get_mut(&instance),
                sigs.get_mut(&instance),
                aars.get_mut(&instance),
            ) {
                (Some(seal), Some(sig), Some(aar)) => {
                    if seal.len() > 1 || sig.len() > 1 || aar.len() > 1 {
                        let mut duplicates = vec![];
                        if seal.len() > 1 {
                            duplicates.push(ARC_SEAL_HEADER_NAME);
                        }
                        if sig.len() > 1 {
                            duplicates.push(ARC_MESSAGE_SIGNATURE_HEADER_NAME);
                        }
                        if aar.len() > 1 {
                            duplicates.push(ARC_AUTHENTICATION_RESULTS_HEADER_NAME);
                        }
                        let duplicates = duplicates.join(", ");
                        issues.push(ARCIssue {
                            reason: format!(
                                "There are duplicate {duplicates} header(s) \
                                    for instance {instance}"
                            ),
                            error: Some(DKIMError::DuplicateARCInstance(instance)),
                            header: None,
                        });
                        continue;
                    }

                    let (seal, seal_header) = seal.pop().expect("one");
                    let (sig, sig_header) = sig.pop().expect("one");
                    let (aar, aar_header) = aar.pop().expect("one");

                    arc_sets.insert(
                        instance,
                        ARCSet {
                            seal,
                            seal_header,
                            sig,
                            sig_header,
                            aar,
                            aar_header,
                        },
                    );
                }
                (None, None, None) => {
                    // Not an error unless there are gaps; we'll check
                    // for that below
                }
                _ => {
                    // One or more are missing
                    issues.push(ARCIssue {
                        reason: format!(
                            "The ARC Set with instance {instance} is \
                                    missing some of its constituent headers"
                        ),
                        error: Some(DKIMError::MissingARCInstance(instance)),
                        header: None,
                    });
                }
            }
        }

        // Ensure that the keys are contiguous
        for instance in 2..=MAX_ARC_INSTANCE {
            if arc_sets.contains_key(&instance) {
                let prior = instance - 1;
                if !arc_sets.contains_key(&prior) {
                    issues.push(ARCIssue {
                        reason: format!("The ARC Set with instance {prior} is missing"),
                        error: Some(DKIMError::MissingARCInstance(prior)),
                        header: None,
                    });
                }
            }
        }

        let mut arc = ARC {
            sets: arc_sets.into_iter().map(|(_k, set)| set).collect(),
            last_validated_instance: 0,
            issues,
        };

        arc.validate_signatures(email, resolver).await;

        arc
    }

    pub async fn validate_signatures(&mut self, email: &ParsedEmail<'_>, resolver: &dyn Resolver) {
        let mut seal_headers = vec![];

        for arc_set in &self.sets {
            if let Err(err) = verify_email_header(
                resolver,
                ARC_MESSAGE_SIGNATURE_HEADER_NAME,
                &arc_set.sig,
                email,
            )
            .await
            {
                self.issues.push(ARCIssue {
                    reason: format!(
                        "The {ARC_MESSAGE_SIGNATURE_HEADER_NAME} for \
                                instance {} failed to validate",
                        arc_set.instance()
                    ),
                    error: Some(err),
                    header: Some(arc_set.sig_header.clone()),
                });

                break;
            }

            seal_headers.push(&arc_set.aar_header);
            seal_headers.push(&arc_set.sig_header);
            // Don't add the seal header yet, as it is implicitly
            // processed by the seal.verify routine

            // Verify the Seal
            if let Err(err) = arc_set.seal.verify(resolver, &seal_headers).await {
                self.issues.push(ARCIssue {
                    reason: format!(
                        "The {ARC_SEAL_HEADER_NAME} for instance {} failed to validate",
                        arc_set.instance()
                    ),
                    error: Some(err),
                    header: Some(arc_set.seal_header.clone()),
                });

                break;
            }
            // now we can add the seal header for any additional passes
            seal_headers.push(&arc_set.seal_header);

            match arc_set.seal.parse_tag::<ChainValidationStatus>("cv") {
                Ok(Some(ChainValidationStatus::Pass)) => {
                    if arc_set.instance() == 1 {
                        self.issues.push(ARCIssue {
                            reason: format!(
                                "The {ARC_SEAL_HEADER_NAME} for instance {} is \
                                marked as cv=pass but that is invalid",
                                arc_set.instance()
                            ),
                            error: None,
                            header: Some(arc_set.seal_header.clone()),
                        });

                        break;
                    }
                }
                Ok(Some(ChainValidationStatus::None)) => {
                    if arc_set.instance() > 1 {
                        self.issues.push(ARCIssue {
                            reason: format!(
                                "The {ARC_SEAL_HEADER_NAME} for instance {} is \
                                marked as cv=none but that is invalid",
                                arc_set.instance()
                            ),
                            error: None,
                            header: Some(arc_set.seal_header.clone()),
                        });

                        break;
                    }
                }
                Ok(Some(ChainValidationStatus::Fail)) => {
                    self.issues.push(ARCIssue {
                        reason: format!(
                            "The {ARC_SEAL_HEADER_NAME} for instance {} is \
                            marked as failing its chain validation",
                            arc_set.instance()
                        ),
                        error: None,
                        header: Some(arc_set.seal_header.clone()),
                    });

                    break;
                }
                Ok(None) => {
                    self.issues.push(ARCIssue {
                        reason: format!(
                            "The {ARC_SEAL_HEADER_NAME} for instance {} is \
                            missing its chain validation status",
                            arc_set.instance()
                        ),
                        error: None,
                        header: Some(arc_set.seal_header.clone()),
                    });
                    break;
                }
                Err(err) => {
                    self.issues.push(ARCIssue {
                        reason: format!(
                            "The {ARC_SEAL_HEADER_NAME} for instance {} has \
                            an invalid chain validation status",
                            arc_set.instance()
                        ),
                        error: Some(err),
                        header: Some(arc_set.seal_header.clone()),
                    });
                    break;
                }
            }

            self.last_validated_instance = arc_set.instance();
        }
    }
}

#[derive(Debug)]
pub struct ARCSet {
    pub aar: ARCAuthenticationResults,
    pub aar_header: Header<'static>,
    pub seal: ARCSealHeader,
    pub seal_header: Header<'static>,
    pub sig: ARCMessageSignatureHeader,
    pub sig_header: Header<'static>,
}

impl ARCSet {
    pub fn instance(&self) -> u8 {
        self.aar.instance
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use dns_resolver::TestResolver;

    const EXAMPLE_MESSAGE: &str = include_str!("../test/arc-example.eml");
    const EXAMPLE_MESSAGE_2: &str = include_str!("../test/arc-example-2.eml");

    const FM3_ZONE_NAME: &str = "fm3._domainkey.messagingengine.com";
    const FM3_ZONE_TXT: &str = "v=DKIM1; k=rsa; p=MIIBIjANBgkqhkiG9w0BAQEFAAOC\
        AQ8AMIIBCgKCAQEA3TntGwdEtmIx+H8Etk1IgA2gLzy9v22TO+BcTUmUFaURWSG413g+VIt\
        m86ntW1bfbgFk/ArrTVAzQxgynoCQky3VXMXl2qEKgGSrLv+QaNvbebVDZI6VZX8D5+aJIN\
        3sCSVY1eXA4x6LbPZ8pAqIAuAhtfXc7rVKbELqlEaUMrQ+ovyjF4R6gfL621BKdLeTF89/k\
        bqJhLwmgtzok6UBUzexDDBhZ0gfGw331J+7aqdJLWUCQv6iE3zkI4myyEcMrgWxRjdZ861x\
        374pNzady/B688A5i4BHoVnBJBuLEYfS1gTCC/7SB6U5AdEin3P0/+DqSH36cu8+MvAZ1C7E2wIDAQAB";

    #[tokio::test]
    async fn test_parse_example_1() {
        let email = ParsedEmail::parse(EXAMPLE_MESSAGE.replace('\n', "\r\n")).unwrap();
        let resolver = TestResolver::default().with_txt(FM3_ZONE_NAME, FM3_ZONE_TXT);
        let arc = ARC::verify(&email, &resolver).await;
        eprintln!("{:#?}", arc.issues);
        assert_eq!(arc.chain_validation_status(), ChainValidationStatus::Pass);
    }

    #[tokio::test]
    async fn test_parse_example_2() {
        let email = ParsedEmail::parse(EXAMPLE_MESSAGE_2.replace('\n', "\r\n")).unwrap();
        let resolver = TestResolver::default().with_txt(FM3_ZONE_NAME, FM3_ZONE_TXT);
        let arc = ARC::verify(&email, &resolver).await;
        eprintln!("{:#?}", arc.issues);
        assert_eq!(arc.chain_validation_status(), ChainValidationStatus::Pass);
    }

    #[tokio::test]
    async fn test_parse_no_sets() {
        let email = ParsedEmail::parse("Subject: hello\r\n\r\nHello\r\n").unwrap();
        let resolver = TestResolver::default();
        let arc = ARC::verify(&email, &resolver).await;
        eprintln!("{:#?}", arc.issues);
        assert_eq!(arc.chain_validation_status(), ChainValidationStatus::None);
    }
}
