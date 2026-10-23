use dns_resolver::{fully_qualify, has_colon_port};
use std::collections::BTreeSet;
use std::fmt::Write;

/// A canonical, lossless summary of the accepted hostname/port set. Preferences
/// remain in the connection plan; input ordering is not part of site identity.
pub(crate) fn factor_names<S: AsRef<str>>(names: &[S]) -> String {
    let mut paths = vec![];
    for name in names {
        let (host, port) = match has_colon_port(name.as_ref()) {
            Some((host, port)) => (host, Some(port)),
            None => (name.as_ref(), None),
        };
        // Keep the resolver's hostname acceptance and normalization behavior.
        if let Ok(name) = fully_qualify(host) {
            let mut labels: Vec<String> = name
                .iter()
                .map(|label| {
                    let mut text = String::new();
                    for &byte in label {
                        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'*') {
                            text.push(byte as char);
                        } else {
                            // Keep literal bytes distinct from grouping syntax,
                            // separators and ports, without backslashes in metric labels.
                            write!(text, "%{byte:02X}").unwrap();
                        }
                    }
                    text
                })
                .collect();
            if let (Some(port), Some(last)) = (port, labels.last_mut()) {
                write!(last, ":{port}").unwrap();
            }
            paths.push(labels);
        }
    }
    // Suffix-first order keeps related branches adjacent without depending on
    // DNS answer order or the preference values of the domain being resolved.
    paths.sort_unstable_by(|a, b| a.iter().rev().cmp(b.iter().rev()));
    paths.dedup();
    let paths: Vec<_> = paths.iter().map(Vec::as_slice).collect();
    match paths.as_slice() {
        [] => String::new(),
        [one] => one.join("."),
        _ => compact_columns(&paths).unwrap_or_else(|| factor_branches(&paths)),
    }
}

/// Preserve the familiar per-label alternations only when their Cartesian
/// product describes exactly the input set. Stop expanding as soon as there are
/// more distinct paths than inputs: alternatives from unrelated hosts can
/// multiply into an enormous product (see `wide_label_product_is_bounded`).
///
/// ```text
/// a.x.test, a.y.test, b.x.test, b.y.test -> (a|b).(x|y).test
/// a.x.test, b.y.test -> None (that form would also include a.y.test and b.x.test)
/// ```
fn compact_columns(paths: &[&[String]]) -> Option<String> {
    // Align labels from the right; None represents a missing leading label
    // that would become an optional component in the compact form.
    let width = paths.iter().map(|path| path.len()).max()?;
    let mut columns: Vec<Vec<Option<&str>>> = vec![vec![]; width];
    for path in paths {
        for (column, label) in columns.iter_mut().zip(
            path.iter()
                .rev()
                .map(|label| Some(label.as_str()))
                .chain(std::iter::repeat(None)),
        ) {
            if !column.contains(&label) {
                column.push(label);
            }
        }
    }

    // Expand the proposed columns only far enough to disprove an exact match.
    let mut expanded = BTreeSet::from([vec![]]);
    for column in columns.iter().rev() {
        let mut next = BTreeSet::new();
        for prefix in &expanded {
            for label in column {
                let mut path = prefix.clone();
                if let Some(label) = label {
                    path.push(*label);
                }
                next.insert(path);
                if next.len() > paths.len() {
                    return None;
                }
            }
        }
        expanded = next;
    }
    let expected: BTreeSet<Vec<&str>> = paths
        .iter()
        .map(|path| path.iter().map(String::as_str).collect())
        .collect();
    if expanded != expected {
        return None;
    }

    // Render the proven-safe alternatives, marking missing labels as optional.
    Some(
        columns
            .iter()
            .rev()
            .map(|column| {
                let labels: Vec<_> = column.iter().filter_map(|label| *label).collect();
                let mut text = match labels.as_slice() {
                    [one] => one.to_string(),
                    _ => format!("({})", labels.join("|")),
                };
                if column.contains(&None) {
                    text.push('?');
                }
                text
            })
            .collect::<Vec<_>>()
            .join("."),
    )
}

/// Factor only complete branches: common ends, then adjacent suffix groups.
/// Unlike independent label alternations, this cannot invent new combinations
/// of labels from different hosts.
///
/// ```text
/// a.x.test, b.x.test, c.y.test -> ((a|b).x|c.y).test
/// mx.a.x.test, mx.b.y.test -> mx.(a.x|b.y).test
/// ```
fn factor_branches(paths: &[&[String]]) -> String {
    if let [one] = paths {
        return one.join(".");
    }
    let mut paths = paths.to_vec();
    // Save the shared prefix, then consume it from each path's remaining slice:
    // [mx.a.x.test, mx.b.y.test] -> prefix [mx], paths [a.x.test, b.y.test].
    let mut prefix = vec![];
    while let Some(first) = paths[0].first() {
        if !paths.iter().all(|path| path.first() == Some(first)) {
            break;
        }
        prefix.push(first.as_str());
        for path in &mut paths {
            *path = &path[1..];
        }
    }
    // Peel off the common suffix in the same way, collecting it right-to-left.
    let mut suffix = vec![];
    while let Some(last) = paths[0].last() {
        if !paths.iter().all(|path| path.last() == Some(last)) {
            break;
        }
        suffix.push(last.as_str());
        for path in &mut paths {
            *path = &path[..path.len() - 1];
        }
    }
    suffix.reverse();

    // A path consisting only of the common ends makes the middle optional:
    // test, a.x.test, b.y.test -> (a.x|b.y)?.test
    let optional = paths.iter().any(|path| path.is_empty());
    paths.retain(|path| !path.is_empty());
    // Suffix-first sorting keeps equal endings adjacent. Recurse on shared
    // endings; otherwise join complete branches without further factoring.
    let groups: Vec<_> = paths.chunk_by(|a, b| a.last() == b.last()).collect();
    let pieces: Vec<_> = if groups.iter().any(|group| group.len() > 1) {
        groups.iter().map(|group| factor_branches(group)).collect()
    } else {
        paths.iter().map(|path| path.join(".")).collect()
    };
    let middle = match pieces.as_slice() {
        [one] if !optional => one.clone(),
        _ => format!("({}){}", pieces.join("|"), if optional { "?" } else { "" }),
    };
    prefix.push(&middle);
    prefix.extend(suffix);
    prefix.join(".")
}

#[cfg(test)]
mod test {
    use super::*;

    /// Expand the notation independently of the formatter. An optional atom
    /// contributes either its labels or no labels; it is not a DNS wildcard.
    fn expand(text: &str) -> BTreeSet<String> {
        fn alternatives(input: &mut &str) -> BTreeSet<Vec<String>> {
            let mut result = sequence(input);
            while let Some(rest) = input.strip_prefix('|') {
                *input = rest;
                result.extend(sequence(input));
            }
            result
        }
        fn sequence(input: &mut &str) -> BTreeSet<Vec<String>> {
            let mut result = BTreeSet::from([vec![]]);
            if input.is_empty() || input.starts_with(['|', ')']) {
                return result;
            }
            loop {
                let values = atom(input);
                let mut next = BTreeSet::new();
                for prefix in &result {
                    for suffix in &values {
                        let mut path = prefix.clone();
                        path.extend(suffix.iter().cloned());
                        next.insert(path);
                        // A broken formatter must fail the test rather than
                        // exhaust memory by expanding an invented product.
                        assert!(next.len() <= 2048, "excessive name expansion");
                    }
                }
                result = next;
                match input.strip_prefix('.') {
                    Some(rest) => *input = rest,
                    None => return result,
                }
            }
        }
        fn atom(input: &mut &str) -> BTreeSet<Vec<String>> {
            let mut values = if let Some(rest) = input.strip_prefix('(') {
                *input = rest;
                let result = alternatives(input);
                *input = input.strip_prefix(')').expect("closing parenthesis");
                result
            } else {
                let end = input.find(['.', '(', ')', '|', '?']).unwrap_or(input.len());
                assert!(end > 0, "empty atom in {input:?}");
                let (label, rest) = input.split_at(end);
                *input = rest;
                BTreeSet::from([vec![label.to_string()]])
            };
            if let Some(rest) = input.strip_prefix('?') {
                *input = rest;
                values.insert(vec![]);
            }
            values
        }
        let mut input = text;
        let result = alternatives(&mut input);
        assert!(input.is_empty(), "unparsed suffix: {input}");
        result.into_iter().map(|labels| labels.join(".")).collect()
    }

    #[test]
    fn familiar_names() {
        assert_eq!(
            factor_names(&[
                "mta5.am0.yahoodns.net",
                "mta6.am0.yahoodns.net",
                "mta7.am0.yahoodns.net"
            ]),
            "(mta5|mta6|mta7).am0.yahoodns.net"
        );
        assert_eq!(
            factor_names(&[
                "gmail-smtp-in.l.google.com",
                "alt1.gmail-smtp-in.l.google.com",
                "alt2.gmail-smtp-in.l.google.com",
                "alt3.gmail-smtp-in.l.google.com",
                "alt4.gmail-smtp-in.l.google.com"
            ]),
            "(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com"
        );
        assert_eq!(factor_names::<&str>(&[]), "");
        assert_eq!(factor_names(&["."]), "");
    }

    #[test]
    fn canonical_destinations() {
        let hosts = ["mx1.example.com", "mx2.example.com"];
        assert_eq!(factor_names(&hosts), factor_names(&[hosts[1], hosts[0]]));
        assert_eq!(
            factor_names(&hosts),
            factor_names(&["MX2.EXAMPLE.COM.", "mx1.example.com.", "mx2.example.com"])
        );
        let mixed = [
            "example-com.mail.protection.outlook.com.",
            "mx-biz.mail.am0.yahoodns.net.",
        ];
        assert_eq!(
            factor_names(&mixed),
            factor_names(&[mixed[1], mixed[0], mixed[1]])
        );
    }

    #[test]
    fn complete_branches_not_label_combinations() {
        let a = ["a.x.targets.test", "b.x.targets.test", "c.y.targets.test"];
        let b = ["a.x.targets.test", "b.y.targets.test", "c.x.targets.test"];
        assert_ne!(factor_names(&a), factor_names(&b));
        assert_eq!(factor_names(&a), "((a|b).x|c.y).targets.test");
        assert_eq!(factor_names(&b), "((a|c).x|b.y).targets.test");
        for hosts in [&a, &b] {
            assert_eq!(
                expand(&factor_names(hosts)),
                hosts.iter().map(|s| s.to_string()).collect()
            );
        }
        let full = ["a.x.test", "a.y.test", "b.x.test", "b.y.test"];
        assert_eq!(factor_names(&full), "(a|b).(x|y).test");
        assert_ne!(factor_names(&full), factor_names(&full[..3]));
    }

    #[test]
    fn google_primary_is_not_invented() {
        let full = [
            "aspmx.l.google.com",
            "alt1.aspmx.l.google.com",
            "alt2.aspmx.l.google.com",
            "aspmx2.googlemail.com",
            "aspmx3.googlemail.com",
        ];
        assert_eq!(
            factor_names(&full),
            "((alt1|alt2)?.aspmx.l.google|(aspmx2|aspmx3).googlemail).com"
        );
        assert_eq!(
            factor_names(&full[1..]),
            "((alt1|alt2).aspmx.l.google|(aspmx2|aspmx3).googlemail).com"
        );
        assert!(!expand(&factor_names(&full[1..])).contains(full[0]));
    }

    #[test]
    fn ports_stay_with_their_hosts() {
        // A domain or routing_domain written as name:port applies that port to
        // every resolved MX host, so mixed ports are synthetic association checks.
        let a = [
            "mx1.example.com:25",
            "mx2.example.com:2525",
            "mx3.example.com:25",
        ];
        let b = [
            "mx1.example.com:25",
            "mx2.example.com:25",
            "mx3.example.com:2525",
        ];
        assert_ne!(factor_names(&a), factor_names(&b));
        for hosts in [&a, &b] {
            assert_eq!(
                expand(&factor_names(hosts)),
                hosts.iter().map(|s| s.to_string()).collect()
            );
        }
        assert_eq!(factor_names(&["MX.EXAMPLE.COM.:025"]), "mx.example.com:25");
        assert_ne!(
            factor_names(&["mx.example.com"]),
            factor_names(&["mx.example.com:25"])
        );
    }

    #[test]
    fn normalization_preserves_hostname_acceptance() {
        assert_eq!(
            factor_names(&["mx.example.com", "http://mail.example.com"]),
            "mx.example.com"
        );
        assert_eq!(factor_names(&["*.example.com"]), "*.example.com");
        assert_eq!(
            factor_names(&["*.example.com", "mx.example.com"]),
            "(*|mx).example.com"
        );
    }

    #[test]
    fn embedded_dots_are_not_label_separators() {
        // DNS labels are length-delimited and may contain a literal dot.
        // These synthetic inputs must not alias names where dots separate labels.
        let name = factor_names(&[r"mx\.backup.example.com"]);
        assert_eq!(name, "mx%2Ebackup.example.com");
        assert_eq!(name, factor_names(&[r"MX\056BACKUP.EXAMPLE.COM."]));
        assert_ne!(name, factor_names(&["mx.backup.example.com"]));

        let hosts = [r"a.x\.y.test", r"b.x\.y.test", "a.z.test", "b.z.test"];
        let name = factor_names(&hosts);
        assert_eq!(name, "(a|b).(x%2Ey|z).test");
        assert_eq!(
            expand(&name),
            ["a.x%2Ey.test", "b.x%2Ey.test", "a.z.test", "b.z.test"]
                .into_iter()
                .map(str::to_string)
                .collect()
        );
        // Site names are also used as metric labels without further escaping.
        assert!(!name.contains(['\\', '"', '\n']));
    }

    #[test]
    fn exhaustive_small_host_sets_round_trip() {
        let universe = [
            "a.x.test",
            "a.y.test",
            "b.x.test",
            "b.y.test",
            "x.test",
            "y.test",
            "test",
            "a.x.test:2525",
        ];
        let mut names = BTreeMap::new();
        for mask in 1..(1 << universe.len()) {
            let hosts: Vec<_> = universe
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, host)| *host)
                .collect();
            let expected: BTreeSet<_> = hosts.iter().map(|s| s.to_string()).collect();
            let name = factor_names(&hosts);
            assert_eq!(expand(&name), expected);
            assert!(
                names.insert(name.clone(), expected).is_none(),
                "colliding name: {name}"
            );
            let mut reordered = hosts.clone();
            for _ in 0..reordered.len() {
                reordered.rotate_left(1);
                assert_eq!(factor_names(&reordered), name);
                reordered.reverse();
                assert_eq!(factor_names(&reordered), name);
            }
        }
    }

    #[test]
    fn wide_label_product_is_bounded() {
        // Independent alternatives at the four varying positions describe 128^4
        // hosts, including a0.b1.c2.d3. The compact-form check must stop before
        // expanding that product and fall back to these complete paths.
        let hosts: Vec<_> = (0..128)
            .map(|i| format!("a{i}.b{i}.c{i}.d{i}.example.com"))
            .collect();
        let name = factor_names(&hosts);
        assert_eq!(expand(&name), hosts.iter().cloned().collect());
    }

    use std::collections::BTreeMap;
}
