use crate::header::{ARCMessageSignatureHeader, ARCSealHeader};
use crate::{verify_email_header, DKIMError, ParsedEmail};
use dns_resolver::Resolver;
use mailparsing::ARCAuthenticationResults;
use std::collections::BTreeMap;

pub const MAX_ARC_INSTANCE: u8 = 50;
pub const ARC_MESSAGE_SIGNATURE_HEADER_NAME: &str = "ARC-Message-Signature";
pub const ARC_SEAL_HEADER_NAME: &str = "ARC-Seal";
pub const ARC_AUTHENTICATION_RESULTS_HEADER_NAME: &str = "ARC-Authentication-Results";

#[derive(Debug)]
pub struct ARCSet {
    pub aar: ARCAuthenticationResults,
    pub seal: ARCSealHeader,
    pub sig: ARCMessageSignatureHeader,
}

impl ARCSet {
    pub fn instance(&self) -> u8 {
        self.aar.instance
    }
}

pub async fn analyze_arc(email: &ParsedEmail<'_>, resolver: &dyn Resolver) {
    let mut seals = BTreeMap::new();
    let mut sigs = BTreeMap::new();
    let mut aars = BTreeMap::new();

    let headers = email.get_headers();
    let mut issues = vec![];

    for hdr in headers.iter_named(ARC_SEAL_HEADER_NAME) {
        match ARCSealHeader::parse(hdr.get_raw_value()) {
            Ok(seal) => {
                let instance = seal.arc_instance().expect("validated by parse");
                seals.entry(instance).or_insert_with(Vec::new).push(seal);
            }
            Err(err) => {
                issues.push(err);
            }
        }
    }

    for hdr in headers.iter_named(ARC_MESSAGE_SIGNATURE_HEADER_NAME) {
        match ARCMessageSignatureHeader::parse(hdr.get_raw_value()) {
            Ok(sig) => {
                let instance = sig.arc_instance().expect("validated by parse");
                sigs.entry(instance).or_insert_with(Vec::new).push(sig);
            }
            Err(err) => {
                issues.push(err);
            }
        }
    }

    for hdr in headers.iter_named(ARC_AUTHENTICATION_RESULTS_HEADER_NAME) {
        match hdr.as_arc_authentication_results() {
            Ok(aar) => {
                if aar.instance == 0 || aar.instance > MAX_ARC_INSTANCE {
                    issues.push(DKIMError::InvalidARCInstance);
                    continue;
                }
                aars.entry(aar.instance).or_insert_with(Vec::new).push(aar);
            }
            Err(err) => {
                issues.push(err.into());
            }
        }
    }

    let mut arc_sets = BTreeMap::new();
    for instance in 1..=MAX_ARC_INSTANCE {
        match (
            seals.get(&instance),
            sigs.get(&instance),
            aars.get(&instance),
        ) {
            (Some(seal), Some(sig), Some(aar)) => {
                if seal.len() > 1 || sig.len() > 1 || aar.len() > 1 {
                    issues.push(DKIMError::DuplicateARCInstance(instance));
                    continue;
                }

                arc_sets.insert(
                    instance,
                    ARCSet {
                        seal: seal[0].clone(),
                        sig: sig[0].clone(),
                        aar: aar[0].clone(),
                    },
                );
            }
            (None, None, None) => {
                // Not an error unless there are gaps; we'll check
                // for that below
            }
            _ => {
                // One or more are missing
                issues.push(DKIMError::MissingARCInstance(instance));
            }
        }
    }

    // Ensure that the keys are contiguous
    for instance in 2..=MAX_ARC_INSTANCE {
        if arc_sets.contains_key(&instance) {
            let prior = instance - 1;
            if !arc_sets.contains_key(&prior) {
                issues.push(DKIMError::MissingARCInstance(prior));
            }
        }
    }

    for arc_set in arc_sets.values() {
        eprintln!("{arc_set:#?}");

        eprintln!("Processing instance {}", arc_set.instance());
        if let Err(err) = verify_email_header(
            resolver,
            ARC_MESSAGE_SIGNATURE_HEADER_NAME,
            &arc_set.sig,
            email,
        )
        .await
        {
            issues.push(err);
        }

        // Verify the Seal
        if let Err(err) =
            verify_email_header(resolver, ARC_SEAL_HEADER_NAME, &arc_set.seal, email).await
        {
            issues.push(err);
        }
    }

    eprintln!("Issues: {issues:#?}");
}

#[cfg(test)]
mod test {
    use super::*;
    use dns_resolver::TestResolver;

    const EXAMPLE_MESSAGE: &str = include_str!("../test/arc-example.eml");

    #[tokio::test]
    async fn test_parse_example_from_rfc8617() {
        let email = ParsedEmail::parse(EXAMPLE_MESSAGE.replace('\n', "\r\n")).unwrap();
        let resolver = TestResolver::default().with_txt(
            "fm3._domainkey.messagingengine.com",
            "v=DKIM1; k=rsa; p=MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA3TntGwdEtmIx+H8Etk1IgA2gLzy9v22TO+BcTUmUFaURWSG413g+VItm86ntW1bfbgFk/ArrTVAzQxgynoCQky3VXMXl2qEKgGSrLv+QaNvbebVDZI6VZX8D5+aJIN3sCSVY1eXA4x6LbPZ8pAqIAuAhtfXc7rVKbELqlEaUMrQ+ovyjF4R6gfL621BKdLeTF89/kbqJhLwmgtzok6UBUzexDDBhZ0gfGw331J+7aqdJLWUCQv6iE3zkI4myyEcMrgWxRjdZ861x374pNzady/B688A5i4BHoVnBJBuLEYfS1gTCC/7SB6U5AdEin3P0/+DqSH36cu8+MvAZ1C7E2wIDAQAB"
        );
        analyze_arc(&email, &resolver).await;
    }
}
