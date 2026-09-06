//! 寫作進度 — how much was written, and against what (Feature #244).
//!
//! Scrivener's targets, counting 字 the way a Chinese publisher counts them.
//! The number a novelist wants is not 「這個檔有多長」 — `:count` answers that
//! — but 「**今天**寫了多少」, and that question cannot be answered from the
//! buffer alone: the manuscript held eighteen thousand 字 this morning too.
//! Somebody has to have written the morning's number down.
//!
//! So the book keeps a log beside itself, at `.yumete/progress.tsv`:
//!
//! ```text
//! # yumete 寫作進度：日期 ⇥ 檔名 ⇥ 起 ⇥ 現
//! 目標 ⇥ 2000
//! 2026-09-05 ⇥ 第一章.md ⇥ 0 ⇥ 1830
//! 2026-09-06 ⇥ 第一章.md ⇥ 1830 ⇥ 2440
//! ```
//!
//! (⇥ is one tab; the file itself holds tabs, not arrows.)
//!
//! **Four fields, and the third is the point.** 起 is what the file held when
//! the day began; 現 is what it holds now. Today's writing is the sum of
//! `現 − 起` over that day's rows, which stays right when the writer deletes a
//! scene (the number goes down, as it should), when several chapters are open
//! at once, and when yesterday's row was never written because the editor was
//! killed. A log of running totals alone would have needed the previous day's
//! row to mean anything, and the day a row is missing every later day is wrong.
//!
//! **It is a text file, in the manuscript's own directory, and a human may
//! edit it.** A writer who moves a chapter, or who wants Monday's number to
//! forget the two thousand 字 pasted in from an old draft, can open it and fix
//! the line. That is worth more than any binary format's tidiness, and it is
//! the same bargain `.yumete/words.txt` makes.
//!
//! Nothing here touches the disk or the clock: [`Log`] parses and prints, and
//! the editor hands it today's date. See [`today`] for how that date is found.

/// One file's writing on one day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// `YYYY-MM-DD`, the local day (see [`today`]).
    pub date: String,
    /// The manuscript's file name — not its path. A book is one directory.
    pub file: String,
    /// 字 the file held when this day's first save happened.
    pub start: usize,
    /// 字 it holds now.
    pub now: usize,
}

impl Row {
    /// `現 − 起`, which is negative on a day spent cutting.
    pub fn written(&self) -> i64 {
        self.now as i64 - self.start as i64
    }
}

/// A book's `.yumete/progress.tsv`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Log {
    /// 每日目標, in 字. `None` until `:target` sets one.
    pub target: Option<usize>,
    /// Every day's rows, oldest first.
    pub rows: Vec<Row>,
}

/// What the file says on its first line, so that whoever opens it knows what
/// they are looking at before they know what yumete is.
const HEADER: &str = "# yumete 寫作進度：日期\t檔名\t起\t現";

/// The first field of the line that carries the target rather than a day.
const TARGET: &str = "目標";

impl Log {
    /// Read a log. **Anything unreadable is dropped, never refused**: this file
    /// is edited by hand, and a writer who fat-fingers one line is owed the
    /// other four hundred rather than an error message.
    pub fn from_text(text: &str) -> Log {
        let mut log = Log::default();
        for line in text.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            if fields[0] == TARGET {
                log.target = fields.get(1).and_then(|n| n.trim().parse().ok());
                continue;
            }
            if fields.len() < 4 {
                continue;
            }
            let (Ok(start), Ok(now)) = (fields[2].trim().parse(), fields[3].trim().parse()) else {
                continue;
            };
            log.rows.push(Row {
                date: fields[0].to_string(),
                file: fields[1].to_string(),
                start,
                now,
            });
        }
        // A hand-edited file may have its days in any order; everything that
        // reads the log — [`Log::days`] above all — is written for the order
        // the log is printed in.
        log.rows.sort_by(|a, b| (&a.date, &a.file).cmp(&(&b.date, &b.file)));
        log
    }

    /// Print it back, oldest day first.
    pub fn to_text(&self) -> String {
        let mut out = String::from(HEADER);
        out.push('\n');
        if let Some(target) = self.target {
            out.push_str(&format!("{TARGET}\t{target}\n"));
        }
        for row in &self.rows {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                row.date, row.file, row.start, row.now
            ));
        }
        out
    }

    /// Record that `file` holds `now` 字 on `date`.
    ///
    /// `opened_with` is what it held when this session opened it, and is used
    /// **only** to open a row that does not exist yet — a file first saved at
    /// four in the afternoon did not have all its 字 written since four. Once
    /// the row exists its 起 is never moved again, so a second save at five
    /// adds to the day rather than starting it over.
    pub fn note(&mut self, date: &str, file: &str, opened_with: usize, now: usize) {
        if let Some(row) = self
            .rows
            .iter_mut()
            .find(|r| r.date == date && r.file == file)
        {
            row.now = now;
            return;
        }
        self.rows.push(Row {
            date: date.to_string(),
            file: file.to_string(),
            // A file that has grown *since* the day began cannot have begun the
            // day longer than it is now: an `opened_with` above `now` is a
            // session that opened the file, cut a scene and saved, and the
            // honest 起 is the one the log can defend.
            start: opened_with.min(now),
            now,
        });
        self.rows.sort_by(|a, b| (&a.date, &a.file).cmp(&(&b.date, &b.file)));
    }

    /// 字 written on `date`, over every file.
    pub fn written_on(&self, date: &str) -> i64 {
        self.rows
            .iter()
            .filter(|r| r.date == date)
            .map(Row::written)
            .sum()
    }

    /// Every day the log holds, oldest first, with what was written on it.
    pub fn days(&self) -> Vec<(String, i64)> {
        let mut days: Vec<(String, i64)> = Vec::new();
        for row in &self.rows {
            match days.last_mut() {
                Some((date, sum)) if *date == row.date => *sum += row.written(),
                _ => days.push((row.date.clone(), row.written())),
            }
        }
        days
    }

    /// 字 the whole book holds, as the log last saw each file.
    ///
    /// **The ledger's answer, not the disk's.** A file the log has never seen
    /// is not in it — counting the directory instead would sweep up the old
    /// draft, the notes and the outline, and a book's length is exactly the
    /// files the writer has been writing.
    pub fn book(&self) -> usize {
        let mut latest: Vec<(&str, usize)> = Vec::new();
        for row in &self.rows {
            match latest.iter_mut().find(|(file, _)| *file == row.file) {
                // The rows are in date order, so the last one wins.
                Some((_, now)) => *now = row.now,
                None => latest.push((&row.file, row.now)),
            }
        }
        latest.iter().map(|(_, now)| now).sum()
    }

    /// How many days in a row have been written on, counting back from `date`.
    ///
    /// **A day is a day the writer added something**, so a day spent cutting
    /// breaks the streak and a day with no row at all breaks it too. Today
    /// itself is allowed to be empty without ending the run — the streak is
    /// something to keep, and at nine in the morning it has not been broken
    /// yet, only not extended.
    pub fn streak(&self, date: &str) -> usize {
        let Some(mut day) = days_from_civil(date) else {
            return 0;
        };
        let mut streak = 0;
        if self.written_on(&civil_from_days(day)) <= 0 {
            day -= 1;
        }
        while self.written_on(&civil_from_days(day)) > 0 {
            streak += 1;
            day -= 1;
        }
        streak
    }
}

/// A day's writing drawn as a bar, `scale` 字 to a full one.
///
/// **Twelve cells, and it is allowed to overflow.** A day that beat the target
/// draws a full bar and says so with a 「＋」 rather than growing past the
/// column and pushing every other day's number out of line — the listing is
/// read down the left edge, and a ragged one cannot be.
pub fn bar(written: i64, scale: usize) -> String {
    const WIDTH: usize = 12;
    if written < 0 {
        return "－".to_string();
    }
    let scale = scale.max(1) as i64;
    let full = (written * WIDTH as i64 / scale).min(WIDTH as i64) as usize;
    let mut bar = "█".repeat(full);
    if written > scale {
        bar.push('＋');
    }
    bar
}

/// The local date as `YYYY-MM-DD`, given the seconds east of UTC.
///
/// The clock is the caller's: [`local_offset`] finds the offset once a session,
/// and the arithmetic here is pure so that a test can ask what happens at
/// midnight without waiting for one.
pub fn today(now_secs: i64, offset_secs: i64) -> String {
    civil_from_days((now_secs + offset_secs).div_euclid(86_400))
}

/// Seconds east of UTC, or `0` when nobody will say.
///
/// **The standard library has no local time**, and a writing log keyed by UTC
/// rolls over at eight in the morning for the writer this editor was built for
/// — which would file a morning's work under yesterday. So the system is asked
/// the way a shell script would ask it. `date +%z` is POSIX and answers
/// `+0800`; Windows has no such thing on the PATH, and there the day still
/// turns at UTC midnight until somebody with a Windows machine wires up
/// `GetTimeZoneInformation`.
pub fn local_offset() -> i64 {
    let Ok(out) = std::process::Command::new("date").arg("+%z").output() else {
        return 0;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    parse_offset(text.trim())
}

/// `+0800` → 28800. Anything else is 0, which is UTC and says so.
fn parse_offset(text: &str) -> i64 {
    let sign = match text.chars().next() {
        Some('+') => 1,
        Some('-') => -1,
        _ => return 0,
    };
    let digits: String = text.chars().skip(1).filter(char::is_ascii_digit).collect();
    if digits.len() < 4 {
        return 0;
    }
    let (Ok(hours), Ok(minutes)) = (digits[0..2].parse::<i64>(), digits[2..4].parse::<i64>()) else {
        return 0;
    };
    sign * (hours * 3600 + minutes * 60)
}

/// Days since the epoch as `YYYY-MM-DD`.
///
/// Hinnant's civil-from-days, which is the whole of the Gregorian calendar in a
/// dozen lines and saves a dependency for one string a day. `yume-ime`'s build
/// script carries the same twelve lines for the same reason.
pub fn civil_from_days(days: i64) -> String {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DD` back to days since the epoch — the inverse, for [`Log::streak`],
/// which has to ask what yesterday was called.
pub fn days_from_civil(date: &str) -> Option<i64> {
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_of_writing_is_the_difference_the_row_holds() {
        let mut log = Log::default();
        log.note("2026-09-06", "第一章.md", 1830, 1830);
        assert_eq!(log.written_on("2026-09-06"), 0);
        // Every later save of the same day moves 現 and leaves 起 alone.
        log.note("2026-09-06", "第一章.md", 1830, 2100);
        log.note("2026-09-06", "第一章.md", 1830, 2440);
        assert_eq!(log.written_on("2026-09-06"), 610);
        assert_eq!(log.rows.len(), 1, "{:?}", log.rows);
    }

    #[test]
    fn a_second_chapter_is_a_second_row_and_the_same_day() {
        let mut log = Log::default();
        log.note("2026-09-06", "第一章.md", 1830, 2440);
        log.note("2026-09-06", "第二章.md", 0, 300);
        assert_eq!(log.written_on("2026-09-06"), 910);
        assert_eq!(log.days(), vec![("2026-09-06".to_string(), 910)]);
    }

    /// A day spent cutting is a negative day, not a zero one: the writer did
    /// the work and the manuscript is shorter, and a log that hid this would
    /// make the week's total a lie.
    #[test]
    fn a_day_of_cutting_counts_backwards() {
        let mut log = Log::default();
        log.note("2026-09-06", "第一章.md", 2440, 2440);
        log.note("2026-09-06", "第一章.md", 2440, 1990);
        assert_eq!(log.written_on("2026-09-06"), -450);
    }

    /// The row is opened by the *first* save of the day, which may be hours in
    /// — so 起 is what the session opened the file with, not what it holds by
    /// the time somebody remembers to save.
    #[test]
    fn the_row_opens_at_what_the_session_opened_the_file_with() {
        let mut log = Log::default();
        log.note("2026-09-06", "第一章.md", 1830, 2440);
        assert_eq!(log.written_on("2026-09-06"), 610);
    }

    /// …but never at more than the file holds. A session that opened a chapter,
    /// cut half of it and saved would otherwise report a day of writing.
    #[test]
    fn a_file_cut_before_its_first_save_does_not_report_a_day_of_writing() {
        let mut log = Log::default();
        log.note("2026-09-06", "第一章.md", 2440, 1990);
        assert_eq!(log.written_on("2026-09-06"), 0);
    }

    #[test]
    fn a_log_survives_the_round_trip_and_a_hand_edit() {
        let mut log = Log::default();
        log.target = Some(2000);
        log.note("2026-09-05", "第一章.md", 0, 1830);
        log.note("2026-09-06", "第一章.md", 1830, 2440);
        let text = log.to_text();
        assert_eq!(Log::from_text(&text), log, "{text}");
        // A hand-edited file with a broken line, a comment and a blank one.
        let mangled = format!("{text}\n# 昨天的\n2026-09-04\t第一章.md\t不知道\n");
        assert_eq!(Log::from_text(&mangled), log);
    }

    #[test]
    fn the_book_is_every_chapter_as_the_log_last_saw_it() {
        let mut log = Log::default();
        log.note("2026-09-05", "第一章.md", 0, 1830);
        log.note("2026-09-06", "第一章.md", 1830, 2440);
        log.note("2026-09-06", "第二章.md", 0, 300);
        assert_eq!(log.book(), 2740);
    }

    #[test]
    fn the_streak_is_days_in_a_row_with_writing_on_them() {
        let mut log = Log::default();
        for (date, start, now) in [
            ("2026-09-01", 0, 500),
            ("2026-09-02", 500, 900),
            // 09-03 missing: the editor was not opened.
            ("2026-09-04", 900, 1400),
            ("2026-09-05", 1400, 1900),
        ] {
            log.note(date, "第一章.md", start, now);
        }
        assert_eq!(log.streak("2026-09-05"), 2);
        // Today, not written on yet, does not break yesterday's run.
        assert_eq!(log.streak("2026-09-06"), 2);
        // The day after that does.
        assert_eq!(log.streak("2026-09-07"), 0);
    }

    #[test]
    fn the_calendar_goes_both_ways() {
        for date in ["1970-01-01", "2026-09-06", "2026-03-01", "2024-02-29"] {
            let days = days_from_civil(date).unwrap();
            assert_eq!(civil_from_days(days), date);
        }
        assert_eq!(days_from_civil("1970-01-01"), Some(0));
        assert_eq!(days_from_civil("不是日期"), None);
    }

    /// The whole reason the offset is looked up at all: a writer in UTC+8 who
    /// writes at nine in the morning must not have it filed under yesterday.
    #[test]
    fn the_day_turns_at_local_midnight() {
        // 2026-09-06 01:00 UTC, which is 09:00 in Shanghai.
        let secs = days_from_civil("2026-09-06").unwrap() * 86_400 + 3_600;
        assert_eq!(today(secs, 8 * 3600), "2026-09-06");
        assert_eq!(today(secs, 0), "2026-09-06");
        // 2026-09-05 23:00 UTC is already the 6th there, and still the 5th here.
        let secs = days_from_civil("2026-09-05").unwrap() * 86_400 + 23 * 3_600;
        assert_eq!(today(secs, 8 * 3600), "2026-09-06");
        assert_eq!(today(secs, 0), "2026-09-05");
    }

    #[test]
    fn a_bar_says_how_the_day_went_without_growing_out_of_its_column() {
        assert_eq!(bar(0, 2000), "");
        assert_eq!(bar(1000, 2000), "██████");
        assert_eq!(bar(2000, 2000), "████████████");
        // Past the target: full, and marked, but still twelve cells of bar.
        assert_eq!(bar(9000, 2000), "████████████＋");
        // A day of cutting is not a bar at all.
        assert_eq!(bar(-450, 2000), "－");
        // A scale of nothing is not a division by zero.
        assert_eq!(bar(300, 0), "████████████＋");
    }

    #[test]
    fn an_offset_nobody_will_state_is_utc() {
        assert_eq!(parse_offset("+0800"), 8 * 3600);
        assert_eq!(parse_offset("-0330"), -(3 * 3600 + 1800));
        assert_eq!(parse_offset("+00:00"), 0);
        assert_eq!(parse_offset(""), 0);
        assert_eq!(parse_offset("CEST"), 0);
    }
}
