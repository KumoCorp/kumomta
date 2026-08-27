use chrono::{DateTime, Datelike, FixedOffset, Offset, TimeZone, Timelike};

const SHORT_WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const SHORT_MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Format `date` as an RFC 2822 `date-time`.
///
/// A year with more than four digits keeps all of them, and a negative year is
/// clamped to `0000`. This keeps a corrupt timestamp printable, where chrono's
/// `DateTime::to_rfc2822` would panic instead.
pub fn format_rfc2822_date<Tz: TimeZone>(date: DateTime<Tz>) -> String {
    // Applying the offset can push a near-limit instant out of NaiveDateTime's
    // range. Fall back to the UTC wall clock at a +0000 offset to keep the
    // printed fields and the printed offset describing the same instant, rather
    // than panic on such a value.
    let zero = FixedOffset::east_opt(0).expect("zero is a valid offset");
    let raw_offset = date.offset().fix();
    let (local, offset) = match date.naive_utc().checked_add_offset(raw_offset) {
        Some(local) => (local, raw_offset),
        None => (date.naive_utc(), zero),
    };

    let weekday = SHORT_WEEKDAYS[local.weekday().num_days_from_sunday() as usize];
    let month = SHORT_MONTHS[local.month0() as usize];
    let year = local.year().max(0);
    let day = local.day();
    let hour = local.hour();
    let minute = local.minute();
    // A leap second is stored as second 59 with a nanosecond past one second.
    // Adding it here renders the 60 that RFC 2822 expects.
    let second = local.second() + local.nanosecond() / 1_000_000_000;

    let offset_seconds = offset.local_minus_utc();
    let (sign, offset_seconds) = if offset_seconds < 0 {
        ('-', -offset_seconds)
    } else {
        ('+', offset_seconds)
    };
    let offset_hours = offset_seconds / 3600;
    let offset_minutes = (offset_seconds % 3600) / 60;

    format!(
        "{weekday}, {day} {month} {year:04} \
         {hour:02}:{minute:02}:{second:02} \
         {sign}{offset_hours:02}{offset_minutes:02}"
    )
}

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

    #[test]
    fn format_representable_date() {
        let date = parse_rfc2822_date("Tue, 1 Jul 2003 10:52:37 +0200").unwrap();
        k9::assert_equal!(format_rfc2822_date(date), "Tue, 1 Jul 2003 10:52:37 +0200");
    }

    #[test]
    fn format_matches_chrono_for_representable_dates() {
        // Our hand-rolled formatter must agree with chrono byte-for-byte over
        // the range chrono can render, across the day/year padding and offset
        // boundaries. Outside that range chrono panics, which is the whole
        // reason we format the field ourselves.
        let utc = FixedOffset::east_opt(0).unwrap();
        let east = FixedOffset::east_opt(2 * 3600).unwrap();
        let west = FixedOffset::west_opt(5 * 3600).unwrap();
        let samples = [
            east.with_ymd_and_hms(2003, 7, 1, 10, 52, 37).unwrap(),
            utc.with_ymd_and_hms(2026, 7, 2, 18, 55, 38).unwrap(),
            west.with_ymd_and_hms(2026, 7, 2, 18, 55, 38).unwrap(),
            utc.with_ymd_and_hms(9999, 12, 31, 23, 59, 59).unwrap(),
            utc.with_ymd_and_hms(1, 1, 1, 0, 0, 0).unwrap(),
        ];
        for date in samples {
            k9::assert_equal!(format_rfc2822_date(date), date.to_rfc2822());
        }
    }

    #[test]
    fn format_far_future_year_keeps_all_digits() {
        // A year past 9999 has no four-digit RFC 2822 form. chrono panics on it;
        // ours renders every digit rather than fail.
        let far_future = chrono::Utc.with_ymd_and_hms(60123, 1, 1, 0, 0, 0).unwrap();
        k9::assert_equal!(
            format_rfc2822_date(far_future),
            "Fri, 1 Jan 60123 00:00:00 +0000"
        );
    }

    #[test]
    fn format_negative_year_clamps_to_zero() {
        let ancient = chrono::Utc.with_ymd_and_hms(-44, 3, 15, 12, 0, 0).unwrap();
        k9::assert_equal!(
            format_rfc2822_date(ancient),
            "Thu, 15 Mar 0000 12:00:00 +0000"
        );
    }

    #[test]
    fn format_offset_overflow_falls_back_to_utc() {
        // An instant at the top of NaiveDateTime's range cannot have a positive
        // offset applied without overflowing. The result must stay internally
        // consistent, keeping the same UTC wall clock and +0000 offset as the
        // plain UTC value rather than pairing the UTC fields with the
        // un-applied offset.
        let max_utc = DateTime::<chrono::Utc>::MAX_UTC;
        let shifted = max_utc.with_timezone(&FixedOffset::east_opt(5 * 3600).unwrap());
        k9::assert_equal!(format_rfc2822_date(shifted), format_rfc2822_date(max_utc));
    }
}
