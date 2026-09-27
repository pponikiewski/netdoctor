//! Local wall-clock time for unix timestamps, without a date crate.

/// Local wall-clock HH:MM for a unix timestamp, without pulling in a date crate.
pub fn format_clock(ts: f64) -> String {
    let secs_of_day = (ts as i64).rem_euclid(86400);
    // SystemTime has no timezone, so ask Windows for the local offset. Asked
    // on every call rather than kept: a kept one would be an hour out for the
    // rest of a run that crosses a daylight-saving change.
    let offset = local_utc_offset_secs();
    let local = (secs_of_day + offset).rem_euclid(86400);
    format!("{:02}:{:02}", local / 3600, (local % 3600) / 60)
}

/// Local date and time safe for a file name: "2026-09-25 14-32".
pub fn file_stamp(ts: f64) -> String {
    let t = ts as i64 + local_utc_offset_secs();
    let (y, m, d) = civil_from_days(t.div_euclid(86400));
    let secs = t.rem_euclid(86400);
    format!("{y:04}-{m:02}-{d:02} {:02}-{:02}", secs / 3600, (secs % 3600) / 60)
}

pub fn format_datetime(ts: f64) -> String {
    let offset = local_utc_offset_secs();
    let t = ts as i64 + offset;
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!("{d:02}.{m:02} {y:04} {:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

/// Time of day to the second, local, for events inside one sitting.
pub fn format_clock_s(ts: f64) -> String {
    let secs = (ts as i64 + local_utc_offset_secs()).rem_euclid(86400);
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

/// Hour of the day, 0-23, in the machine's own time zone. Grouping outages by
/// hour only says anything if the hour is the one the user lives in.
pub fn local_hour(ts: f64) -> i64 {
    (ts as i64 + local_utc_offset_secs()).div_euclid(3600).rem_euclid(24)
}

fn local_utc_offset_secs() -> i64 {
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
}
