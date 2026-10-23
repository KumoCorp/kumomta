//! This module was generated automatically by running
//! `(cd crates/mailparsing/codegen && cargo run --release)`
//! Do not modify by hand!
//! Its source can be found in crates/mailparsing/codegen/src/main.rs
//!
//! The table maps an obsolete alphabetic time zone abbreviation, uppercased, to
//! its numeric RFC 2822 offset. It is derived from the IANA time zone database
//! via chrono-tz: an abbreviation is included only when the database renders it
//! at a single offset across the year 2026 (the year when this file
//! was last generated). Abbreviations the database renders at several offsets
//! (`IST`, `CST`, ...) are absent by construction and fall back to the -0000
//! unknown offset when parsing.

/// Maps an uppercased alphabetic zone abbreviation to a timezone offset.
pub static ZONE_OFFSETS: phf::Map<&'static str, &'static str> = ::phf::Map {
    key: 2453081474412133617,
    disps: &[
        (0, 6),
        (2, 22),
        (26, 19),
        (0, 0),
        (0, 0),
        (27, 19),
        (0, 16),
        (35, 4),
        (2, 11),
    ],
    entries: &[
        ("WAT", "+0100"),
        ("PKT", "+0500"),
        ("BST", "+0100"),
        ("NZDT", "+1300"),
        ("ACDT", "+1030"),
        ("CET", "+0100"),
        ("KST", "+0900"),
        ("MSK", "+0300"),
        ("EDT", "-0400"),
        ("AWST", "+0800"),
        ("SAST", "+0200"),
        ("HKT", "+0800"),
        ("AEDT", "+1100"),
        ("EET", "+0200"),
        ("CHST", "+1000"),
        ("UTC", "+0000"),
        ("WET", "+0000"),
        ("MDT", "-0600"),
        ("IDT", "+0300"),
        ("EEST", "+0300"),
        ("JST", "+0900"),
        ("GMT", "+0000"),
        ("NDT", "-0230"),
        ("EST", "-0500"),
        ("CAT", "+0200"),
        ("MST", "-0700"),
        ("AKST", "-0900"),
        ("CEST", "+0200"),
        ("HDT", "-0900"),
        ("AST", "-0400"),
        ("NZST", "+1200"),
        ("HST", "-1000"),
        ("NST", "-0330"),
        ("WEST", "+0100"),
        ("AEST", "+1000"),
        ("AKDT", "-0800"),
        ("ACST", "+0930"),
        ("WIT", "+0900"),
        ("SST", "-1100"),
        ("PDT", "-0700"),
        ("WITA", "+0800"),
        ("WIB", "+0700"),
        ("ADT", "-0300"),
        ("EAT", "+0300"),
    ],
};
