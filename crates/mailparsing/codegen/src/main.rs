use chrono::{Datelike, NaiveDate, Offset, TimeZone, Utc};
use chrono_tz::{OffsetName, Tz, TZ_VARIANTS};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufWriter, Write};

// This program generates mailparsing/src/zone_offsets.rs, a perfect hash map
// from an obsolete alphabetic time zone abbreviation to its numeric RFC 2822
// offset. It is consumed when parsing a `date-time` whose zone chrono itself
// does not recognize.
//
// Run this like this: `cd crates/mailparsing/codegen ; cargo run --release`

fn format_offset(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let magnitude = seconds.abs();
    let hours = magnitude / 3600;
    let minutes = (magnitude % 3600) / 60;
    format!("{sign}{hours:02}{minutes:02}")
}

fn main() {
    // The tz database records how each zone's abbreviation and offset have
    // changed over its whole history, but we only want each abbreviation's
    // present-day meaning. We therefore sample the offsets of the current
    // calendar year: every month of a year is a valid date whose rules the tz
    // database knows (past months from history, later months from its projected
    // rules), so twelve monthly probes catch both the standard and the summer
    // offset of a zone while a long-past offset change (Moscow was +0400 until
    // 2014, for example) does not count against it as ambiguity. Reading the
    // year from the clock keeps the table current with no year to hardcode and
    // let go stale.
    let sample_year = Utc::now().year();

    // Every distinct offset each abbreviation names during the sample year. An
    // abbreviation observed with more than one offset is ambiguous.
    let mut offsets: BTreeMap<String, BTreeSet<i32>> = BTreeMap::new();

    for tz in TZ_VARIANTS.iter() {
        let tz: Tz = *tz;
        for month in 1..=12 {
            let utc = NaiveDate::from_ymd_opt(sample_year, month, 1)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap();
            let offset = tz.offset_from_utc_datetime(&utc);
            let Some(abbr) = offset.abbreviation() else {
                continue;
            };
            // The fallback only rewrites purely alphabetic tokens; tzdb also
            // emits numeric abbreviations such as "+03" and single-letter
            // military zones, neither of which we resolve here.
            if abbr.len() < 2 || !abbr.bytes().all(|b| b.is_ascii_alphabetic()) {
                continue;
            }
            offsets
                .entry(abbr.to_ascii_uppercase())
                .or_default()
                .insert(offset.fix().local_minus_utc());
        }
    }

    // The tz database is the sole authority for what an abbreviation means: an
    // abbreviation it renders at a single offset during the year is emitted,
    // and one it renders at several (`IST` is +0100/+0200/+0530) is dropped as
    // ambiguous in the data itself, falling back to the -0000 unknown offset
    // rather than guessing one region's meaning. An abbreviation some other
    // region also uses in human speech but which the database prints
    // numerically for that region (Bangladesh prints "+06", not "BST") is not
    // ambiguous here: the only alphabetic form the database attests is the one
    // we emit, and any tz-consistent sender writes the numeric offset for the
    // other region.
    let entries: Vec<(String, String)> = offsets
        .iter()
        .filter(|(_abbr, seconds)| seconds.len() == 1)
        .map(|(abbr, seconds)| {
            let offset = format_offset(*seconds.iter().next().unwrap());
            (abbr.clone(), format!("\"{offset}\""))
        })
        .collect();

    let mut map = phf_codegen::Map::new();
    for (abbr, value) in &entries {
        map.entry(abbr.as_str(), value.as_str());
    }
    let built = map.build();
    let mut file = BufWriter::new(File::create("../src/zone_offsets.rs").unwrap());
    write!(
        &mut file,
        r#"//! This module was generated automatically by running
//! `(cd crates/mailparsing/codegen && cargo run --release)`
//! Do not modify by hand!
//! Its source can be found in crates/mailparsing/codegen/src/main.rs
//!
//! The table maps an obsolete alphabetic time zone abbreviation, uppercased, to
//! its numeric RFC 2822 offset. It is derived from the IANA time zone database
//! via chrono-tz: an abbreviation is included only when the database renders it
//! at a single offset across the year {sample_year} (the year when this file
//! was last generated). Abbreviations the database renders at several offsets
//! (`IST`, `CST`, ...) are absent by construction and fall back to the -0000
//! unknown offset when parsing.

/// Maps an uppercased alphabetic zone abbreviation to a timezone offset.
pub static ZONE_OFFSETS: phf::Map<&'static str, &'static str> = {map};
"#,
        sample_year = sample_year,
        map = built
    )
    .unwrap();

    eprintln!(
        "wrote {} zone abbreviations to ../src/zone_offsets.rs",
        entries.len()
    );
}
