use chrono::{DateTime, FixedOffset};

/// Parse an RFC 2822 `date-time`, tolerating obsolete alphabetic time zones.
///
/// chrono's `parse_from_rfc2822` accepts the zone tokens named in RFC 5322
/// section 4.3 (`UT`, `GMT`, the North American `EST`/`EDT`/... set) but
/// rejects any other alphabetic zone. Real senders still emit some: Amazon SES
/// writes its DSN dates with the token `UTC`, which chrono does not recognize.
///
/// When the strict parse fails we retry after replacing a trailing alphabetic
/// zone token with a numeric offset. A token with a single widely-agreed
/// meaning (`UTC`, and the common regional abbreviations chrono lacks such as
/// `CET`/`CEST`) resolves to its real offset. Anything chrono neither knows nor
/// we can resolve unambiguously falls back to `-0000`, the RFC 5322 section 4.3
/// unknown-offset marker: the same instant as UTC, which keeps the stated
/// wall-clock date and time while recording that the true offset is unknown.
/// The original error is preserved when the retry does not help, so genuinely
/// malformed input still reports the real problem rather than a zone complaint.
pub fn parse_rfc2822_date(
    input: &str,
) -> Result<DateTime<FixedOffset>, chrono::format::ParseError> {
    DateTime::parse_from_rfc2822(input).or_else(|err| {
        if let Some(rewritten) = rewrite_unknown_alphabetic_zone(input) {
            if let Ok(date) = DateTime::parse_from_rfc2822(&rewritten) {
                return Ok(date);
            }
        }
        Err(err)
    })
}

/// If `input` ends with an alphabetic zone token, return a copy with that token
/// replaced by a numeric offset; otherwise return `None`.
///
/// CFWS (comments and folding whitespace) is stripped with the crate's parser
/// first, so a trailing comment such as `(Coordinated)` does not hide the zone.
/// A recognized abbreviation resolves to its offset; any other alphabetic token
/// becomes the `-0000` unknown offset.
fn rewrite_unknown_alphabetic_zone(input: &str) -> Option<String> {
    let cleaned = crate::rfc5322_parser::strip_cfws(input)?;
    // strip_cfws joins non-empty tokens with single spaces, so the token after
    // the final space is never empty.
    let (head, zone) = cleaned.rsplit_once(' ')?;
    if !zone.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let offset = named_zone_offset(zone).unwrap_or("-0000");
    Some(format!("{head} {offset}"))
}

/// Return the numeric UTC offset, in RFC 2822 `+HHMM` form, for an alphabetic
/// zone abbreviation, or `None` when it is unknown or ambiguous.
///
/// The mapping comes from the generated `zone_offsets` table.
fn named_zone_offset(zone: &str) -> Option<&'static str> {
    crate::zone_offsets::ZONE_OFFSETS
        .get(zone.to_ascii_uppercase().as_str())
        .copied()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn obsolete_utc_zone() {
        let ses = parse_rfc2822_date("Thu, 02 Jul 26 18:55:38 UTC").unwrap();
        let canonical = parse_rfc2822_date("Thu, 02 Jul 2026 18:55:38 +0000").unwrap();
        k9::assert_equal!(ses, canonical);
    }

    #[test]
    fn recognized_zones_keep_their_offset() {
        // chrono already handles these; the fallback must not intercept them.
        let eastern = parse_rfc2822_date("Thu, 02 Jul 26 18:55:38 EST").unwrap();
        k9::assert_equal!(eastern.to_rfc3339(), "2026-07-02T18:55:38-05:00");

        let numeric = parse_rfc2822_date("Tue, 1 Jul 2003 10:52:37 +0200").unwrap();
        k9::assert_equal!(numeric.to_rfc3339(), "2003-07-01T10:52:37+02:00");
    }

    #[test]
    fn regional_zone_resolves_to_its_offset() {
        // Common abbreviations chrono rejects are resolved to their real offset.
        let cest = parse_rfc2822_date("Thu, 02 Jul 26 18:55:38 CEST").unwrap();
        k9::assert_equal!(cest.to_rfc3339(), "2026-07-02T18:55:38+02:00");

        let jst = parse_rfc2822_date("Thu, 02 Jul 26 18:55:38 JST").unwrap();
        k9::assert_equal!(jst.to_rfc3339(), "2026-07-02T18:55:38+09:00");
    }

    #[test]
    fn ambiguous_zone_becomes_unknown_offset() {
        // The tz database itself renders `IST` at three offsets (Ireland
        // +0100, Israel +0200, India +0530), so it is ambiguous in the data
        // and we cannot resolve it. Per RFC 5322 section 4.3 it keeps the
        // stated wall-clock time at the -0000 unknown offset, which is the same
        // instant as UTC.
        let ist = parse_rfc2822_date("Thu, 02 Jul 26 18:55:38 IST").unwrap();
        let utc = parse_rfc2822_date("Thu, 02 Jul 26 18:55:38 UTC").unwrap();
        k9::assert_equal!(ist, utc);
    }

    #[test]
    fn trailing_zone_comment_is_ignored() {
        let with_comment = parse_rfc2822_date("Thu, 02 Jul 26 18:55:38 UTC (Coordinated)").unwrap();
        let canonical = parse_rfc2822_date("Thu, 02 Jul 2026 18:55:38 +0000").unwrap();
        k9::assert_equal!(with_comment, canonical);
    }

    #[test]
    fn garbage_still_errors() {
        parse_rfc2822_date("not a date").unwrap_err();
    }
}
