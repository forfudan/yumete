//! The wall clock, to the second, as fourteen digits.
//!
//! One job: name a file after **when** it was made, so a second picture of the
//! same page does not have to argue with the first (#189). A date in the name
//! is the whole of what this module is for, which is why it answers a string
//! and not a time — nothing in the editor has any use for arithmetic on it.
//!
//! **Local time, not UTC.** The name is read by a person looking through their
//! downloads folder, and 「下午三點那張」 is how they will look for it. That
//! costs a call into the platform (`localtime_r`, `GetLocalTime`), because the
//! standard library knows the instant and not the offset.

/// The local wall clock as `yyyymmddhhmmss`.
///
/// Fourteen digits, always — the caller puts them straight into a file name.
pub fn stamp() -> String {
    let (y, mo, d, h, mi, s) = now();
    format!("{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}")
}

/// Year, month, day, hour, minute, second — local where the platform will say.
#[cfg(unix)]
fn now() -> (i64, u32, u32, u32, u32, u32) {
    // SAFETY: `time` reads the clock and `localtime_r` fills a `tm` we own.
    // The `_r` form is the one that writes into that `tm` rather than into a
    // static another thread could be reading.
    unsafe {
        let seconds = libc::time(std::ptr::null_mut());
        let mut broken: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&seconds, &mut broken).is_null() {
            // No zone the platform will admit to. UTC is wrong by hours; a
            // file with no name at all is wrong by the whole file.
            return civil(seconds as i64);
        }
        (
            broken.tm_year as i64 + 1900,
            broken.tm_mon as u32 + 1,
            broken.tm_mday as u32,
            broken.tm_hour as u32,
            broken.tm_min as u32,
            // 60 on a leap second, which is a real second and a legal name.
            broken.tm_sec as u32,
        )
    }
}

#[cfg(windows)]
fn now() -> (i64, u32, u32, u32, u32, u32) {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: the call only writes the `SYSTEMTIME` we hand it.
    unsafe {
        let mut t = std::mem::zeroed::<windows_sys::Win32::Foundation::SYSTEMTIME>();
        GetLocalTime(&mut t);
        (
            t.wYear as i64,
            t.wMonth as u32,
            t.wDay as u32,
            t.wHour as u32,
            t.wMinute as u32,
            t.wSecond as u32,
        )
    }
}

#[cfg(not(any(unix, windows)))]
fn now() -> (i64, u32, u32, u32, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    civil(secs)
}

/// A count of seconds since 1970 as a date and a time, in UTC.
///
/// Howard Hinnant's `civil_from_days`, which the standard library does not
/// export and which is short enough to keep honest here. It is the fallback
/// for a platform that will not say what its zone is, and the one piece of
/// this module a test can hold still.
fn civil(seconds: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    // Count from March, so the leap day is the last day of the year and the
    // month lengths fall into a pattern with no exception in the middle.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = match mp < 10 {
        true => mp + 3,
        false => mp - 9,
    } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        year,
        month,
        day,
        (rest / 3600) as u32,
        (rest / 60 % 60) as u32,
        (rest % 60) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one piece with a right answer written down elsewhere.
    #[test]
    fn the_fallback_clock_agrees_with_dates_anyone_can_check() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_000_000_000), (2001, 9, 9, 1, 46, 40));
        // The end of a leap day, which is where a wrong month length shows.
        assert_eq!(civil(951_868_799), (2000, 2, 29, 23, 59, 59));
        // Before the epoch, because `div_euclid` is the whole reason for it.
        assert_eq!(civil(-1), (1969, 12, 31, 23, 59, 59));
    }

    /// Fourteen digits is the contract; a file name is built out of it.
    #[test]
    fn a_stamp_is_fourteen_digits() {
        let said = stamp();
        assert_eq!(said.len(), 14, "{said}");
        assert!(said.chars().all(|c| c.is_ascii_digit()), "{said}");
        // The year is this century — a clock that answered 1970 would pass
        // every test above and still name every picture the same thing.
        assert!(said.starts_with("20"), "{said}");
    }
}
