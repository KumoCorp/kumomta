use phf_codegen::Set;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Write};
use titlecase::Titlecase;

// This program generates mod-smtp-response-normalize/src/dict.rs
// which contains a perfect hash set that can be used to determine
// if an input string is a dictionary word.
// It uses the locally installed `/usr/share/dict/words` file as
// the source of its dictionary, supplemented by a few words that
// are commonly used in SMTP.
//
// Run this like this: `cd crates/mod-smtp-response-normalize/codegen ; cargo run --release`

static EXTRA_WORDS: &[&str] = &["SpamCop"];

static ACRONYMS: &[&str] = &[
    "arc", "bimi", "dkim", "dmarc", "dns", "rbl", "spf", "surbl", "uuid",
];

fn add_word(stage: &mut HashSet<String>, word: &str) {
    stage.insert(word.to_string());

    let titled = word.titlecase();
    if titled != word {
        stage.insert(titled);
    }
}

fn add_acronym(stage: &mut HashSet<String>, word: &str) {
    stage.insert(word.to_lowercase());
    stage.insert(word.to_uppercase());
}

fn main() {
    let words = std::fs::read_to_string("/usr/share/dict/words").unwrap();

    let mut stage = HashSet::new();

    for word in words.lines() {
        add_word(&mut stage, word);
    }

    for word in EXTRA_WORDS {
        add_word(&mut stage, word);
    }

    for word in ACRONYMS {
        add_acronym(&mut stage, word);
    }

    let mut set = Set::new();
    for word in stage.iter() {
        set.entry(word);
    }

    let mut file = BufWriter::new(File::create("../src/dict.rs").unwrap());
    write!(
        &mut file,
        r#"
//! This module was generated automatically by running
//! `(cd crates/mod-smtp-response-normalize/codegen && cargo run --release)`
//! Do not modify by hand!
//! Its source can be found in crates/mod-smtp-response-normalize/codegen/src/main.rs

pub static DICT: phf::Set<&'static str> = {};
"#,
        set.build()
    )
    .unwrap();
}
