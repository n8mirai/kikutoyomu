use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn now_unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Howard Hinnant's civil_from_days: converts days since 1970-01-01 to (year, month, day).
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year as i32, m as u32, d as u32)
}

/// Howard Hinnant's days_from_civil: converts (year, month, day) to days since 1970-01-01.
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64 - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m = m as u64;
    let d = d as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

pub fn format_utc_rfc3339(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86400);
    let rem = unix_secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, hh, mm, ss)
}

pub fn now_utc_rfc3339() -> String {
    format_utc_rfc3339(now_unix_secs())
}

/// Parse an RFC3339 UTC timestamp like "2026-11-23T14:56:00Z" (or with fractional seconds) into unix seconds.
pub fn parse_rfc3339_unix(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let y: i32 = s.get(0..4)?.parse().ok()?;
    let mo: u32 = s.get(5..7)?.parse().ok()?;
    let d: u32 = s.get(8..10)?.parse().ok()?;
    let hh: i64 = s.get(11..13)?.parse().ok()?;
    let mm: i64 = s.get(14..16)?.parse().ok()?;
    let ss: i64 = s.get(17..19)?.parse().ok()?;
    let days = days_from_civil(y, mo, d);
    Some(days * 86400 + hh * 3600 + mm * 60 + ss)
}

#[cfg(windows)]
pub fn local_offset_secs() -> i64 {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};

    let mut utc = SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };
    let mut loc = utc;
    unsafe {
        GetSystemTime(&mut utc);
        GetLocalTime(&mut loc);
    }
    let utc_s = days_from_civil(utc.wYear as i32, utc.wMonth as u32, utc.wDay as u32) * 86400
        + utc.wHour as i64 * 3600
        + utc.wMinute as i64 * 60
        + utc.wSecond as i64;
    let loc_s = days_from_civil(loc.wYear as i32, loc.wMonth as u32, loc.wDay as u32) * 86400
        + loc.wHour as i64 * 3600
        + loc.wMinute as i64 * 60
        + loc.wSecond as i64;
    // Round to nearest 15-minute offset to eliminate any sub-second boundary drift
    let diff = loc_s - utc_s;
    ((diff + if diff >= 0 { 450 } else { -450 }) / 900) * 900
}

#[cfg(not(windows))]
pub fn local_offset_secs() -> i64 {
    0
}

pub fn local_day_index(unix_secs: i64) -> i64 {
    (unix_secs + local_offset_secs()).div_euclid(86400)
}

pub fn today_local_day_index() -> i64 {
    local_day_index(now_unix_secs())
}
