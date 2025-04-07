use chrono::naive::NaiveTime;
use chrono::{DateTime, Datelike, FixedOffset, LocalResult, TimeZone, Timelike, Utc, Weekday};
use chrono_tz::Tz;
use kumo_chrono_helper::*;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct DaysOfWeek: u8 {
        const MON = 1;
        const TUE = 2;
        const WED = 4;
        const THU = 8;
        const FRI = 16;
        const SAT = 32;
        const SUN = 64;
    }
}

impl From<Weekday> for DaysOfWeek {
    fn from(day: Weekday) -> DaysOfWeek {
        match day {
            Weekday::Mon => DaysOfWeek::MON,
            Weekday::Tue => DaysOfWeek::TUE,
            Weekday::Wed => DaysOfWeek::WED,
            Weekday::Thu => DaysOfWeek::THU,
            Weekday::Fri => DaysOfWeek::FRI,
            Weekday::Sat => DaysOfWeek::SAT,
            Weekday::Sun => DaysOfWeek::SUN,
        }
    }
}

/// Represents a restriction on when the message can be sent.
/// This encodes the permitted times.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Copy)]
pub struct ScheduleRestriction {
    #[serde(rename = "dow")]
    pub days_of_week: DaysOfWeek,
    #[serde(rename = "tz")]
    pub timezone: Tz,
    pub start: NaiveTime,
    pub end: NaiveTime,
}

impl ScheduleRestriction {
    fn start_end_on_day(&self, dt: DateTime<Tz>) -> Option<(DateTime<Tz>, DateTime<Tz>)> {
        let y = dt.year();
        let m = dt.month();
        let d = dt.day();

        let start = match self.timezone.with_ymd_and_hms(
            y,
            m,
            d,
            self.start.hour(),
            self.start.minute(),
            self.start.second(),
        ) {
            LocalResult::Single(t) => t,
            _ => return None,
        };

        let end = match self.timezone.with_ymd_and_hms(
            y,
            m,
            d,
            self.end.hour(),
            self.end.minute(),
            self.end.second(),
        ) {
            LocalResult::Single(t) => t,
            _ => return None,
        };
        Some((start, end))
    }
}

#[derive(Debug, Serialize, Clone, PartialEq, Copy)]
pub struct Scheduling {
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub restriction: Option<ScheduleRestriction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_attempt: Option<DateTime<FixedOffset>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<DateTime<FixedOffset>>,
}

/// serde usually does a great job, but for the case of an optional
/// flattened value, it will silently swallow any errors that should
/// have been raised from parsing the fields it contained, and convert
/// that to a None value.
/// That's very undesirable, and unfortunately for us, it means having
/// to manually implement Deserialize, so that is what this big chunk
/// of code is.
impl<'de> Deserialize<'de> for Scheduling {
    fn deserialize<D>(deserializer: D) -> Result<Self, <D as serde::Deserializer<'de>>::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        /// Our visitor; we'll collect an optional value for
        /// each possible field here
        #[derive(Default)]
        struct V {
            // Note that this is serialized as "dow"
            days_of_week: Option<DaysOfWeek>,
            // Note that this is serialized as "tz"
            timezone: Option<Tz>,
            start: Option<NaiveTime>,
            end: Option<NaiveTime>,
            first_attempt: Option<DateTime<FixedOffset>>,
            expires: Option<DateTime<FixedOffset>>,
        }

        fn do_value<'de, T: Deserialize<'de>, M: serde::de::MapAccess<'de>>(
            map: &mut M,
            label: &str,
            target: &mut Option<T>,
        ) -> Result<(), M::Error> {
            match map.next_value() {
                Err(err) => Err(M::Error::custom(format!("{label}: {err:#}"))),
                Ok(v) => {
                    target.replace(v);
                    Ok(())
                }
            }
        }

        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Scheduling;

            fn expecting(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
                fmt.write_str("a map containing the Scheduling spec")
            }

            fn visit_map<M>(mut self, mut map: M) -> Result<Scheduling, M::Error>
            where
                M: serde::de::MapAccess<'de>,
            {
                // We must iterate borrowed keys here for broader compatibility.
                // Even though retrieving the keys as `&str` functions directly
                // here in the unit tests for this module, it will fail at
                // runtime when processing lua values, as lua uses Cow<_,str>.
                use std::borrow::Cow;

                while let Some(k) = map.next_key::<Cow<'_, str>>()? {
                    match k.as_ref() {
                        "dow" => {
                            do_value(&mut map, "dow", &mut self.days_of_week)?;
                        }
                        "tz" => {
                            do_value(&mut map, "tz", &mut self.timezone)?;
                        }
                        "start" => {
                            do_value(&mut map, "start", &mut self.start)?;
                        }
                        "end" => {
                            do_value(&mut map, "end", &mut self.end)?;
                        }
                        "first_attempt" => {
                            do_value(&mut map, "first_attempt", &mut self.first_attempt)?;
                        }
                        "expires" => {
                            do_value(&mut map, "expires", &mut self.expires)?;
                        }
                        _ => {
                            return Err(M::Error::custom(format!("invalid field name {k}")));
                        }
                    }
                }

                // We require all of the fields of the restriction to be present,
                // or none of them. If only some are present, that is an error,
                // and we must report it.
                let restriction = match (self.days_of_week, self.timezone, self.start, self.end) {
                    (Some(days_of_week), Some(timezone), Some(start), Some(end)) => {
                        if start > end {
                            return Err(M::Error::custom(format!(
                                "'start' must be before 'end' and define a time window \
                                 within a given day. start={start:?}, end={end:?}"
                            )));
                        }
                        Some(ScheduleRestriction {
                            days_of_week,
                            timezone,
                            start,
                            end,
                        })
                    }
                    (None, None, None, None) => None,
                    (days_of_week, timezone, start, end) => {
                        let mut missing = vec![];
                        if days_of_week.is_none() {
                            missing.push("dow");
                        }
                        if timezone.is_none() {
                            missing.push("tz");
                        }
                        if start.is_none() {
                            missing.push("start");
                        }
                        if end.is_none() {
                            missing.push("end");
                        }
                        let is_are = if missing.len() == 1 { "is" } else { "are" };
                        return Err(M::Error::custom(format!(
                            "scheduling restrictions requires all restriction \
                                    fields to be present. {} {is_are} missing",
                            missing.join(", ")
                        )));
                    }
                };

                Ok(Scheduling {
                    restriction,
                    first_attempt: self.first_attempt,
                    expires: self.expires,
                })
            }
        }

        deserializer.deserialize_any(V::default())
    }
}

impl Scheduling {
    pub fn adjust_for_schedule(&self, mut dt: DateTime<Utc>) -> DateTime<Utc> {
        if let Some(start) = &self.first_attempt {
            if dt < *start {
                dt = (*start).into();
            }
        }

        if let Some(restrict) = &self.restriction {
            let mut dt = dt.with_timezone(&restrict.timezone);

            let one_day = chrono::Duration::try_days(1).expect("always able to represent 1 day");

            // Worst case is 1 week off the current time; if we
            // can't find a time in a reasonable number of iterations,
            // something is wrong!
            for _ in 0..8 {
                let weekday = dt.weekday();
                let dow: DaysOfWeek = weekday.into();

                let (start, end) = match restrict.start_end_on_day(dt) {
                    Some(result) => result,
                    None => {
                        // Wonky date/time, try the next day
                        dt += one_day;
                        continue;
                    }
                };

                if restrict.days_of_week.contains(dow) {
                    if dt < start {
                        // Delay until the start time
                        dt = start;
                        break;
                    }

                    if dt < end {
                        // We're within the permitted range
                        break;
                    }
                }

                // Try the same start time the next day
                dt = start + one_day;
            }
            dt.with_timezone(&Utc)
        } else {
            dt
        }
    }

    pub fn is_within_schedule(&self, dt: DateTime<Utc>) -> bool {
        if let Some(start) = &self.first_attempt {
            if dt < *start {
                return false;
            }
        }

        if let Some(restrict) = &self.restriction {
            let dt = dt.with_timezone(&restrict.timezone);

            let weekday: DaysOfWeek = dt.weekday().into();

            if !restrict.days_of_week.contains(weekday) {
                return false;
            }

            let (start, end) = match restrict.start_end_on_day(dt) {
                Some(result) => result,
                None => return false,
            };

            if dt < start {
                return false;
            }
            if dt >= end {
                return false;
            }
        }

        true
    }
}

const DAYS: &[(&str, DaysOfWeek)] = &[
    ("Monday", DaysOfWeek::MON),
    ("Tuesday", DaysOfWeek::TUE),
    ("Wednesday", DaysOfWeek::WED),
    ("Thursday", DaysOfWeek::THU),
    ("Friday", DaysOfWeek::FRI),
    ("Saturday", DaysOfWeek::SAT),
    ("Sunday", DaysOfWeek::SUN),
];

impl FromStr for DaysOfWeek {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let mut days = DaysOfWeek::empty();
        'next: for dow in s.split(',') {
            for (label, value) in DAYS {
                if dow.eq_ignore_ascii_case(label) || dow.eq_ignore_ascii_case(&label[0..3]) {
                    days.set(*value, true);
                    continue 'next;
                }
            }
            return Err(format!("invalid day '{dow}'"));
        }

        Ok(days)
    }
}

impl Serialize for DaysOfWeek {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut result = String::new();
        for (label, value) in DAYS {
            if self.contains(*value) {
                if !result.is_empty() {
                    result.push(',');
                }
                result.push_str(&label[0..3]);
            }
        }
        serializer.serialize_str(&result)
    }
}

impl<'de> Deserialize<'de> for DaysOfWeek {
    fn deserialize<D>(deserializer: D) -> Result<DaysOfWeek, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visit {}
        impl serde::de::Visitor<'_> for Visit {
            type Value = DaysOfWeek;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a comma separated list of days of the week like 'Mon,Tue'")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                value.parse::<DaysOfWeek>().map_err(|err| E::custom(err))
            }
        }

        deserializer.deserialize_str(Visit {})
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn days_of_week() {
        let all = "Mon,Tue,Wed,Thu,Fri,Sat,Sun".parse::<DaysOfWeek>().unwrap();
        k9::assert_equal!(
            all,
            DaysOfWeek::MON
                | DaysOfWeek::TUE
                | DaysOfWeek::WED
                | DaysOfWeek::THU
                | DaysOfWeek::FRI
                | DaysOfWeek::SAT
                | DaysOfWeek::SUN
        );

        let middle = "Wed,Tue,Thursday".parse::<DaysOfWeek>().unwrap();
        k9::assert_equal!(middle, DaysOfWeek::TUE | DaysOfWeek::WED | DaysOfWeek::THU);

        k9::assert_equal!(
            "Wed,Sumday".parse::<DaysOfWeek>().unwrap_err(),
            "invalid day 'Sumday'"
        );
    }

    #[test]
    fn schedule_parse_restriction() {
        let sched = Scheduling {
            restriction: Some(ScheduleRestriction {
                days_of_week: DaysOfWeek::MON | DaysOfWeek::WED,
                timezone: "America/Phoenix".parse().unwrap(),
                start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                end: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            }),
            first_attempt: None,
            expires: None,
        };

        let serialized = serde_json::to_string(&sched).unwrap();
        k9::snapshot!(
            &serialized,
            r#"{"dow":"Mon,Wed","tz":"America/Phoenix","start":"09:00:00","end":"17:00:00"}"#
        );

        let round_trip: Scheduling = serde_json::from_str(&serialized).unwrap();
        k9::assert_equal!(sched, round_trip);
    }

    #[test]
    fn schedule_bogus_date() {
        k9::snapshot!(
            serde_json::from_str::<Scheduling>(r#"{"first_attempt":"09:00:00"}"#,).unwrap_err(),
            r#"Error("first_attempt: input contains invalid characters", line: 1, column: 27)"#
        );
    }

    #[test]
    fn schedule_parse_missing_parts() {
        k9::snapshot!(
            serde_json::from_str::<Scheduling>(
                r#"{"tz":"America/Phoenix","end":"17:00:00","start":"09:00:00"}"#,
            )
            .unwrap_err(),
            r#"Error("scheduling restrictions requires all restriction fields to be present. dow is missing", line: 1, column: 60)"#
        );

        k9::snapshot!(
            serde_json::from_str::<Scheduling>(r#"{"tz":"America/Phoenix","start":"09:00:00"}"#,)
                .unwrap_err(),
            r#"Error("scheduling restrictions requires all restriction fields to be present. dow, end are missing", line: 1, column: 43)"#
        );
    }

    #[test]
    fn schedule_parse_restriction_and_start() {
        let sched = Scheduling {
            restriction: Some(ScheduleRestriction {
                days_of_week: DaysOfWeek::MON | DaysOfWeek::WED,
                timezone: "America/Phoenix".parse().unwrap(),
                start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                end: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            }),
            first_attempt: DateTime::parse_from_rfc3339("1996-12-19T16:39:57-08:00").ok(),
            expires: None,
        };

        let serialized = serde_json::to_string(&sched).unwrap();
        k9::snapshot!(
            &serialized,
            r#"{"dow":"Mon,Wed","tz":"America/Phoenix","start":"09:00:00","end":"17:00:00","first_attempt":"1996-12-19T16:39:57-08:00"}"#
        );

        let round_trip: Scheduling = serde_json::from_str(&serialized).unwrap();
        k9::assert_equal!(sched, round_trip);

        let _: Scheduling = serde_json::from_str(
            r#"{"dow":"Mon","tz":"America/Phoenix","end":"17:00:00","start":"09:00:00"}"#,
        )
        .unwrap();
    }

    #[test]
    fn schedule_parse_no_restriction_and_start() {
        let sched = Scheduling {
            restriction: None,
            first_attempt: DateTime::parse_from_rfc3339("1996-12-19T16:39:57-08:00").ok(),
            expires: None,
        };

        let serialized = serde_json::to_string(&sched).unwrap();
        k9::snapshot!(
            &serialized,
            r#"{"first_attempt":"1996-12-19T16:39:57-08:00"}"#
        );

        let round_trip: Scheduling = serde_json::from_str(&serialized).unwrap();
        k9::assert_equal!(sched, round_trip);
    }

    #[test]
    fn schedule_adjust_start() {
        let sched = Scheduling {
            restriction: None,
            first_attempt: DateTime::parse_from_rfc3339("2023-03-20T16:39:57-08:00").ok(),
            expires: None,
        };

        let now: DateTime<Utc> = DateTime::parse_from_rfc3339("2023-03-20T08:00:00-08:00")
            .unwrap()
            .into();
        k9::assert_equal!(sched.adjust_for_schedule(now), sched.first_attempt.unwrap());
    }

    #[test]
    fn schedule_adjust_dow() {
        let phoenix: Tz = "America/Phoenix".parse().unwrap();
        let sched = Scheduling {
            restriction: Some(ScheduleRestriction {
                days_of_week: DaysOfWeek::MON | DaysOfWeek::WED,
                timezone: phoenix,
                start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                end: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            }),
            first_attempt: None,
            expires: None,
        };

        // This is a Tuesday
        let now: DateTime<Utc> = DateTime::parse_from_rfc3339("2023-03-28T08:00:00-08:00")
            .unwrap()
            .into();

        let adjusted = sched.adjust_for_schedule(now).with_timezone(&phoenix);
        // Expected to round into wednesday, the next day
        k9::assert_equal!(adjusted.to_string(), "2023-03-29 09:00:00 MST");
    }

    #[test]
    fn schedule_adjust_dow_2() {
        let phoenix: Tz = "America/Phoenix".parse().unwrap();
        let sched = Scheduling {
            restriction: Some(ScheduleRestriction {
                days_of_week: DaysOfWeek::MON | DaysOfWeek::FRI,
                timezone: phoenix,
                start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                end: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            }),
            first_attempt: None,
            expires: None,
        };

        // This is a Monday, but after hours
        let now: DateTime<Utc> = DateTime::parse_from_rfc3339("2023-03-27T18:00:00-08:00")
            .unwrap()
            .into();

        let adjusted = sched.adjust_for_schedule(now).with_timezone(&phoenix);
        // Expected to round into Friday, later that week
        k9::assert_equal!(adjusted.to_string(), "2023-03-31 09:00:00 MST");
    }

    #[test]
    fn start_after_end() {
        k9::snapshot!(serde_json::from_str::<Scheduling>(
            r#"{"dow":"Mon,Tue,Wed,Thu,Fri,Sat,Sun","tz":"Etc/UTC","end":"20:57:49","start":"21:10:49", "first_attempt":"2025-03-14T21:10:49Z"}"#,
        )
        .unwrap_err(), r#"Error("'start' must be before 'end' and define a time window within a given day. start=21:10:49, end=20:57:49", line: 1, column: 128)"#);
    }
}
