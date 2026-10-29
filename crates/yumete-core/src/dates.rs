//! **Dates and times that `C-a` and `C-x` step**, as helix's do (2026-10-29,
//! measured against helix 25.07.1).
//!
//! A selection that is exactly one of the forms below changes; anything else,
//! a malformed or impossible date among them (`2024-02-30`, `2024-1-5`,
//! `Mon Nov 24 2021` on a Wednesday), is left alone. A date steps by days, a
//! time or a date with a time by minutes, its seconds kept. A time alone goes
//! round the clock: `23:59` then `C-a` is `00:00`.
//!
//! helix uses chrono for this. Its forms are few and fixed, so they are read
//! and written here by hand, and no dependency is added for them.

/// helix's list, in its order. The first form the whole text fits wins.
const FORMS: &[&str] = &[
    "%Y-%m-%d %H:%M:%S",
    "%Y/%m/%d %H:%M:%S",
    "%Y-%m-%d %H:%M",
    "%Y/%m/%d %H:%M",
    "%Y-%m-%d",
    "%Y/%m/%d",
    "%a %b %d %Y",
    "%d-%b-%Y",
    "%Y %b %d",
    "%b %d, %Y",
    "%-I:%M:%S %P",
    "%-I:%M %P",
    "%-I:%M:%S %p",
    "%-I:%M %p",
    "%H:%M:%S",
    "%H:%M",
];

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// What a form read out of the text. `None` is a field the form does not have.
#[derive(Default)]
struct Read {
    year: Option<i64>,
    month: Option<i64>,
    day: Option<i64>,
    weekday: Option<i64>,
    hour: Option<i64>,
    minute: Option<i64>,
    second: Option<i64>,
    /// The 12-hour clock's hour and whether it is after noon.
    hour12: Option<i64>,
    pm: Option<bool>,
}

/// Whether `text` is one of the forms, whatever stepping it would give: a
/// date at the edge of the four-digit years still steps the other way.
pub fn reads(text: &str) -> bool {
    FORMS.iter().any(|form| read(text, form).is_some())
}

/// `text` stepped by `by` days or minutes, written in the form it came in, or
/// `None` when it is not a date or a time.
pub fn stepped(text: &str, by: i64) -> Option<String> {
    FORMS.iter().find_map(|form| {
        let read = read(text, form)?;
        write(form, &step(&read, by)?)
    })
}

/// The pieces of a form: `%x` fields and the literal characters between them.
fn pieces(form: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = form;
    while !rest.is_empty() {
        let n = match rest {
            _ if rest.starts_with("%-") => 3,
            _ if rest.starts_with('%') => 2,
            _ => rest.chars().next().map_or(1, char::len_utf8),
        };
        out.push(&rest[..n]);
        rest = &rest[n..];
    }
    out
}

fn read(text: &str, form: &str) -> Option<Read> {
    let mut r = Read::default();
    let mut rest = text;
    let digits = |rest: &mut &str, n: usize| -> Option<i64> {
        let head = rest.get(..n)?;
        if !head.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *rest = &rest[n..];
        head.parse().ok()
    };
    let name = |rest: &mut &str, names: &[&str]| -> Option<i64> {
        let i = names.iter().position(|n| rest.starts_with(n))?;
        *rest = &rest[names[i].len()..];
        Some(i as i64)
    };
    for piece in pieces(form) {
        match piece {
            "%Y" => r.year = Some(digits(&mut rest, 4)?),
            "%m" => r.month = Some(digits(&mut rest, 2)?),
            "%d" => r.day = Some(digits(&mut rest, 2)?),
            "%H" => r.hour = Some(digits(&mut rest, 2)?),
            "%M" => r.minute = Some(digits(&mut rest, 2)?),
            "%S" => r.second = Some(digits(&mut rest, 2)?),
            "%a" => r.weekday = Some(name(&mut rest, &WEEKDAYS)?),
            "%b" => r.month = Some(name(&mut rest, &MONTHS)? + 1),
            // No leading zero: helix leaves `07:21 AM` alone.
            "%-I" => {
                let n = rest.bytes().take(2).take_while(u8::is_ascii_digit).count();
                if n == 0 || rest.starts_with('0') {
                    return None;
                }
                r.hour12 = Some(digits(&mut rest, n)?);
            }
            "%p" => r.pm = Some(name(&mut rest, &["AM", "PM"])? == 1),
            "%P" => r.pm = Some(name(&mut rest, &["am", "pm"])? == 1),
            literal => rest = rest.strip_prefix(literal)?,
        }
    }
    if !rest.is_empty() {
        return None;
    }
    if let (Some(h), Some(pm)) = (r.hour12, r.pm) {
        if !(1..=12).contains(&h) {
            return None;
        }
        r.hour = Some(h % 12 + if pm { 12 } else { 0 });
    }
    let fits = |v: Option<i64>, top: i64| v.is_none_or(|v| (0..=top).contains(&v));
    if !fits(r.hour, 23) || !fits(r.minute, 59) || !fits(r.second, 59) {
        return None;
    }
    if let (Some(y), Some(m), Some(d)) = (r.year, r.month, r.day) {
        if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
            return None;
        }
        if r.weekday.is_some_and(|w| w != weekday(days_from_civil(y, m, d))) {
            return None;
        }
    }
    Some(r)
}

/// Days for a date alone, minutes for anything with a time.
fn step(r: &Read, by: i64) -> Option<Read> {
    let mut out = Read { second: r.second, ..Read::default() };
    let minutes = |r: &Read| r.hour.unwrap_or(0) * 60 + r.minute.unwrap_or(0);
    match (r.year, r.minute) {
        (Some(y), None) => {
            let day = days_from_civil(y, r.month?, r.day?).checked_add(by)?;
            out.set_date(day);
        }
        (Some(y), Some(_)) => {
            let at = (days_from_civil(y, r.month?, r.day?) * 1440 + minutes(r)).checked_add(by)?;
            out.set_date(at.div_euclid(1440));
            out.set_time(at.rem_euclid(1440));
        }
        (None, Some(_)) => out.set_time((minutes(r) + by.rem_euclid(1440)).rem_euclid(1440)),
        (None, None) => return None,
    }
    // Four digits, as it was read.
    out.year.is_none_or(|y| (0..=9999).contains(&y)).then_some(out)
}

impl Read {
    fn set_date(&mut self, day: i64) {
        let (y, m, d) = civil_from_days(day);
        (self.year, self.month, self.day, self.weekday) = (Some(y), Some(m), Some(d), Some(weekday(day)));
    }

    fn set_time(&mut self, minute_of_day: i64) {
        let hour = minute_of_day / 60;
        (self.hour, self.minute) = (Some(hour), Some(minute_of_day % 60));
        self.hour12 = Some(match hour % 12 {
            0 => 12,
            h => h,
        });
        self.pm = Some(hour >= 12);
    }
}

fn write(form: &str, r: &Read) -> Option<String> {
    let mut out = String::new();
    for piece in pieces(form) {
        match piece {
            "%Y" => out += &format!("{:04}", r.year?),
            "%m" => out += &format!("{:02}", r.month?),
            "%d" => out += &format!("{:02}", r.day?),
            "%H" => out += &format!("{:02}", r.hour?),
            "%M" => out += &format!("{:02}", r.minute?),
            "%S" => out += &format!("{:02}", r.second?),
            "%a" => out += WEEKDAYS[r.weekday? as usize],
            "%b" => out += MONTHS[r.month? as usize - 1],
            "%-I" => out += &r.hour12?.to_string(),
            "%p" => out += if r.pm? { "PM" } else { "AM" },
            "%P" => out += if r.pm? { "pm" } else { "am" },
            literal => out += literal,
        }
    }
    Some(out)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// 0 is Sunday; 1970-01-01 was a Thursday.
fn weekday(day: i64) -> i64 {
    (day + 4).rem_euclid(7)
}

#[cfg(test)]
mod tests {
    use super::stepped;

    /// Every row was read off helix 25.07.1 (`x_` then `C-a`, `C-x` or a
    /// count first).
    #[test]
    fn dates_and_times_step_as_helix_steps_them() {
        for (text, by, want) in [
            ("2024-01-31", 1, Some("2024-02-01")),
            ("2024/02/28", 1, Some("2024/02/29")),
            ("2023-12-31 23:59:30", 1, Some("2024-01-01 00:00:30")),
            ("2024-01-31 23:59", 1, Some("2024-02-01 00:00")),
            ("2024/01/31 10:00", 1, Some("2024/01/31 10:01")),
            ("Wed Nov 24 2021", 1, Some("Thu Nov 25 2021")),
            ("24-Nov-2021", 1, Some("25-Nov-2021")),
            ("2021 Nov 24", 1, Some("2021 Nov 25")),
            ("Nov 24, 2021", 1, Some("Nov 25, 2021")),
            ("7:21:53 am", 1, Some("7:22:53 am")),
            ("11:59 pm", 1, Some("12:00 am")),
            ("12:00 pm", 1, Some("12:01 pm")),
            ("7:21 AM", 1, Some("7:22 AM")),
            ("23:59:59", 1, Some("00:00:59")),
            ("23:59", 1, Some("00:00")),
            ("23:59", 3, Some("00:02")),
            ("12:30 am", -31, Some("11:59 pm")),
            ("1999-12-31", 1, Some("2000-01-01")),
            ("2024-01-31", -1, Some("2024-01-30")),
            ("2024-03-01", -5, Some("2024-02-25")),
            ("2024-1-5", 1, None),
            ("2024-02-30", 1, None),
            ("12:5", 1, None),
            ("nov 24, 2021", 1, None),
            ("Mon Nov 24 2021", 1, None),
            ("Nov 5, 2021", 1, None),
            ("13:00 pm", 1, None),
            ("0:30 am", 1, None),
            ("07:21 AM", 1, None),
            ("24:00", 1, None),
            ("2024-02-29 12:00:61", 1, None),
            ("12", 1, None),
        ] {
            assert_eq!(stepped(text, by).as_deref(), want, "{text} by {by}");
        }
    }
}
