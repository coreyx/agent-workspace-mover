//! UTC timestamps for records, the log and move ids, without a date library.

use std::time::{SystemTime, UNIX_EPOCH};

/// A moment in UTC, to the second.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Utc {
    /// Seconds since 1970-01-01T00:00:00Z.
    seconds: u64,
}

impl Utc {
    /// The current moment.
    pub fn now() -> Self {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Self { seconds }
    }

    /// The moment `seconds` after the Unix epoch.
    pub fn from_unix(seconds: u64) -> Self {
        Self { seconds }
    }

    /// Seconds since the Unix epoch.
    pub fn unix(self) -> u64 {
        self.seconds
    }

    /// The calendar date and time: year, month, day, hour, minute, second.
    fn parts(self) -> (i64, u32, u32, u32, u32, u32) {
        let days = (self.seconds / 86_400) as i64;
        let in_day = self.seconds % 86_400;
        // Howard Hinnant's days-to-civil algorithm, for the proleptic Gregorian calendar.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = yoe + era * 400 + i64::from(month <= 2);
        (
            year,
            month,
            day,
            (in_day / 3_600) as u32,
            (in_day / 60 % 60) as u32,
            (in_day % 60) as u32,
        )
    }

    /// RFC 3339, for records: `2026-10-23T09:57:01Z`.
    pub fn rfc3339(self) -> String {
        let (y, mo, d, h, mi, s) = self.parts();
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
    }

    /// Compact, for move ids and file names: `20261023-095701`.
    pub fn compact(self) -> String {
        let (y, mo, d, h, mi, s) = self.parts();
        format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
    }

    /// The date alone, for daily log files: `20261023`.
    pub fn date(self) -> String {
        let (y, mo, d, ..) = self.parts();
        format!("{y:04}{mo:02}{d:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::Utc;

    #[test]
    fn formats_known_moments() {
        assert_eq!(Utc::from_unix(0).rfc3339(), "1970-01-01T00:00:00Z");
        assert_eq!(
            Utc::from_unix(951_782_400).rfc3339(),
            "2000-02-29T00:00:00Z",
            "a leap day"
        );
        assert_eq!(
            Utc::from_unix(1_792_749_421).rfc3339(),
            "2026-10-23T09:57:01Z"
        );
        assert_eq!(Utc::from_unix(1_792_749_421).compact(), "20261023-095701");
        assert_eq!(Utc::from_unix(1_792_749_421).date(), "20261023");
        assert_eq!(
            Utc::from_unix(4_102_444_799).rfc3339(),
            "2099-12-31T23:59:59Z"
        );
        assert_eq!(Utc::from_unix(1_792_749_421).unix(), 1_792_749_421);
    }

    #[test]
    fn now_is_after_this_was_written() {
        assert!(Utc::now() > Utc::from_unix(1_792_700_000));
    }
}
