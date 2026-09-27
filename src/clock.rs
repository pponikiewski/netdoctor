//! Local wall-clock time for unix timestamps, without a date crate.

/// Local wall-clock HH:MM for a unix timestamp, without pulling in a date crate.
pub fn format_clock(ts: f64) -> String {
    let local = (ts as i64 + local_offset_at(ts)).rem_euclid(86400);
    format!("{:02}:{:02}", local / 3600, (local % 3600) / 60)
}

/// Local date and time safe for a file name: "2026-09-25 14-32".
pub fn file_stamp(ts: f64) -> String {
    let t = ts as i64 + local_offset_at(ts);
    let (y, m, d) = civil_from_days(t.div_euclid(86400));
    let secs = t.rem_euclid(86400);
    format!("{y:04}-{m:02}-{d:02} {:02}-{:02}", secs / 3600, (secs % 3600) / 60)
}

pub fn format_datetime(ts: f64) -> String {
    let t = ts as i64 + local_offset_at(ts);
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!("{d:02}.{m:02} {y:04} {:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

/// Time of day to the second, local, for events inside one sitting.
pub fn format_clock_s(ts: f64) -> String {
    let secs = (ts as i64 + local_offset_at(ts)).rem_euclid(86400);
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

/// Hour of the day, 0-23, in the machine's own time zone. Grouping outages by
/// hour only says anything if the hour is the one the user lives in.
pub fn local_hour(ts: f64) -> i64 {
    (ts as i64 + local_offset_at(ts)).div_euclid(3600).rem_euclid(24)
}

/// Seconds to add to a unix timestamp to get the machine's local time at
/// that moment, daylight saving included.
///
/// Taken from the timestamp's own date, not from today: an outage from July
/// read in December happened at the hour it happened, not an hour earlier,
/// and the report built from it is shown to someone who can check.
fn local_offset_at(ts: f64) -> i64 {
    offset_on_date(ts).unwrap_or_else(offset_now)
}

// ponytail: the zone's current yearly daylight-saving rule is applied to every
// year, so a zone that has changed its rule since the event is out by that
// change. Upgrade path: SystemTimeToTzSpecificLocalTimeEx with
// GetDynamicTimeZoneInformation, which reads the historical rules from the
// registry, and so costs a registry read per timestamp drawn.
fn offset_on_date(ts: f64) -> Option<i64> {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{
        FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime,
    };
    // FILETIME counts 100 ns ticks from 1601-01-01.
    const TICKS_PER_S: i64 = 10_000_000;
    const UNIX_FROM_1601_S: i64 = 11_644_473_600;

    let utc_ticks = (ts as i64).checked_add(UNIX_FROM_1601_S)?.checked_mul(TICKS_PER_S)?;
    let utc_ticks = u64::try_from(utc_ticks).ok()?;
    let utc_ft =
        FILETIME { dwLowDateTime: utc_ticks as u32, dwHighDateTime: (utc_ticks >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    let mut local_ft = FILETIME::default();
    // SAFETY: every pointer is to a local that outlives the call.
    unsafe {
        FileTimeToSystemTime(&utc_ft, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
        SystemTimeToFileTime(&local, &mut local_ft).ok()?;
    }
    let local_ticks =
        (u64::from(local_ft.dwHighDateTime) << 32) | u64::from(local_ft.dwLowDateTime);
    Some((local_ticks as i64 - utc_ticks as i64) / TICKS_PER_S)
}

/// Today's offset. Only a fallback for a timestamp Windows would not convert,
/// which is one far outside any date this app records.
fn offset_now() -> i64 {
    use windows::Win32::System::Time::GetTimeZoneInformation;
    use windows::Win32::System::Time::TIME_ZONE_INFORMATION;
    unsafe {
        let mut tz = TIME_ZONE_INFORMATION::default();
        let rc = GetTimeZoneInformation(&mut tz);
        // Bias is minutes to ADD to local to get UTC, so invert it.
        let extra = match rc {
            2 => tz.DaylightBias, // TIME_ZONE_ID_DAYLIGHT
            _ => tz.StandardBias,
        };
        -((tz.Bias + extra) as i64) * 60
    }
}

/// Howard Hinnant's days-from-civil, inverted. Avoids a date dependency.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_date_conversion_matches_known_dates() {
        // 2024-02-29 was day 19782 since the epoch.
        assert_eq!(civil_from_days(19782), (2024, 2, 29));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn a_timestamp_is_shown_in_the_offset_of_its_own_date() {
        // The bug this guards: every timestamp took today's offset, so in a
        // zone with daylight saving a July outage read in winter was an hour
        // early in the history and in the report.
        use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};

        // 2026-01-15 and 2026-07-15, both 12:00 UTC.
        let (jan, jul) = (1_768_478_400.0, 1_784_116_800.0);
        assert_eq!(civil_from_days(jan as i64 / 86400), (2026, 1, 15));
        assert_eq!(civil_from_days(jul as i64 / 86400), (2026, 7, 15));

        assert!(offset_on_date(jan).is_some(), "Windows refused to convert an ordinary date");
        let (winter, summer) = (local_offset_at(jan), local_offset_at(jul));

        let mut tz = TIME_ZONE_INFORMATION::default();
        // SAFETY: the pointer is to a local that outlives the call.
        unsafe { GetTimeZoneInformation(&mut tz) };
        if tz.StandardDate.wMonth == 0 {
            assert_eq!(winter, summer, "a zone without daylight saving has one offset");
        } else {
            let shift = -i64::from(tz.DaylightBias) * 60;
            assert_eq!((summer - winter).abs(), shift.abs(), "winter {winter}, summer {summer}");
        }
    }
}
