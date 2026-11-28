//! **Numbers as a heading spells them**: `1`, `一`, `壹`, `a`, `A`, `i`, `I`
//! — the counting symbols a Typst numbering pattern uses, so one pattern
//! language serves both kinds of file. Written by `:heading-numbering`, read
//! back by it and by `:sort-sections -k number`.
//!
//! Chinese is written as a book writes it: 十一 (not 一十一), 二十, 一百零三,
//! 一百一十; the financial forms always say the 壹 of 壹拾 (壹拾壹,
//! 壹佰壹拾). A zero standing alone — the place of a level a heading skipped
//! — is 〇 in the everyday forms and 零 in the financial ones. No 廿 or 卅.

/// One counting symbol of a numbering pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counting {
    /// `1`: 1, 2, 3.
    Arabic,
    /// `一`: 一, 二 … 十一.
    Chinese,
    /// `壹`: 壹, 贰 … 壹拾壹 (貳, 參 in traditional script).
    Financial,
    /// `a`: a … z, aa.
    LowerLatin,
    /// `A`: A … Z, AA.
    UpperLatin,
    /// `i`: i, ii, iii.
    LowerRoman,
    /// `I`: I, II, III.
    UpperRoman,
}

const EVERYDAY: [char; 10] = ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
const FINANCIAL_SIMPLE: [char; 10] = ['零', '壹', '贰', '叁', '肆', '伍', '陆', '柒', '捌', '玖'];
const FINANCIAL_TRAD: [char; 10] = ['零', '壹', '貳', '參', '肆', '伍', '陸', '柒', '捌', '玖'];

impl Counting {
    /// The counting symbol `c` stands for, if it is one.
    pub fn of(c: char) -> Option<Counting> {
        Some(match c {
            '1' => Counting::Arabic,
            '一' => Counting::Chinese,
            '壹' => Counting::Financial,
            'a' => Counting::LowerLatin,
            'A' => Counting::UpperLatin,
            'i' => Counting::LowerRoman,
            'I' => Counting::UpperRoman,
            _ => return None,
        })
    }

    /// `n` written in this system; `traditional` picks 貳參陸 over 贰叁陆
    /// (and 萬 over 万).
    pub fn write(self, n: u64, traditional: bool) -> String {
        match self {
            Counting::Arabic => n.to_string(),
            Counting::Chinese => chinese(n, &EVERYDAY, ['千', '百', '十'], traditional, false),
            Counting::Financial => {
                let digits = if traditional { &FINANCIAL_TRAD } else { &FINANCIAL_SIMPLE };
                chinese(n, digits, ['仟', '佰', '拾'], traditional, true)
            }
            Counting::LowerLatin => latin(n, b'a'),
            Counting::UpperLatin => latin(n, b'A'),
            Counting::LowerRoman => roman(n).to_lowercase(),
            Counting::UpperRoman => roman(n),
        }
    }

    /// Whether `c` can be part of a number written in this system.
    pub fn spells(self, c: char) -> bool {
        match self {
            Counting::Arabic => c.is_ascii_digit(),
            Counting::Chinese => "〇零一二两兩三四五六七八九十百千万萬亿億".contains(c),
            Counting::Financial => "零壹贰貳叁參肆伍陆陸柒捌玖拾佰仟万萬亿億".contains(c),
            Counting::LowerLatin => c.is_ascii_lowercase(),
            Counting::UpperLatin => c.is_ascii_uppercase(),
            Counting::LowerRoman => "ivxlcdm".contains(c),
            Counting::UpperRoman => "IVXLCDM".contains(c),
        }
    }

    /// The number `text` spells in this system, when it is one.
    pub fn read(self, text: &str) -> Option<u64> {
        if text.is_empty() || !text.chars().all(|c| self.spells(c)) {
            return None;
        }
        match self {
            Counting::Arabic => text.parse().ok(),
            Counting::Chinese | Counting::Financial => read_chinese(text),
            Counting::LowerLatin | Counting::UpperLatin => {
                let base = if self == Counting::LowerLatin { b'a' } else { b'A' };
                text.bytes().try_fold(0u64, |n, b| n.checked_mul(26)?.checked_add(u64::from(b - base) + 1))
            }
            Counting::LowerRoman | Counting::UpperRoman => {
                let n = read_roman(&text.to_uppercase())?;
                // Only the spelling this writes: `iiii` is no number of ours.
                (roman(n) == text.to_uppercase()).then_some(n)
            }
        }
    }
}

/// `n` in Chinese numerals, four digits to a group under 万 and 亿.
fn chinese(n: u64, digits: &[char; 10], units: [char; 3], traditional: bool, financial: bool) -> String {
    if n == 0 {
        return digits[0].to_string();
    }
    let groups = [(n / 100_000_000) % 10_000, (n / 10_000) % 10_000, n % 10_000];
    let big = [if traditional { '億' } else { '亿' }, if traditional { '萬' } else { '万' }];
    let mut out = String::new();
    // Anything above a hundred million million is past what a heading counts.
    if n >= 1_000_000_000_000 {
        return n.to_string();
    }
    let mut zero = false;
    for (g, &group) in groups.iter().enumerate() {
        if group == 0 {
            zero |= !out.is_empty();
            continue;
        }
        if !out.is_empty() && (zero || group < 1000) {
            out.push('零');
        }
        zero = false;
        let place = [group / 1000, (group / 100) % 10, (group / 10) % 10, group % 10];
        let mut gap = false;
        let mut said = false;
        for (p, &d) in place.iter().enumerate() {
            if d == 0 {
                gap |= said;
                continue;
            }
            if gap {
                out.push('零');
                gap = false;
            }
            out.push(digits[d as usize]);
            if p < 3 {
                out.push(units[p]);
            }
            said = true;
        }
        if g < 2 {
            out.push(big[g]);
        }
    }
    // 十一, not 一十一 — in the everyday forms, and only when the 十 leads.
    if !financial && (10..20).contains(&n) {
        out.remove(0);
    }
    out
}

/// Chinese numerals read back, 十一 and 一十一 alike, 两 as 二.
fn read_chinese(text: &str) -> Option<u64> {
    let digit = |c: char| -> Option<u64> {
        EVERYDAY
            .iter()
            .position(|&d| d == c)
            .or_else(|| FINANCIAL_SIMPLE.iter().position(|&d| d == c))
            .or_else(|| FINANCIAL_TRAD.iter().position(|&d| d == c))
            .map(|d| d as u64)
            .or(match c {
                '两' | '兩' => Some(2),
                _ => None,
            })
    };
    let (mut total, mut section, mut part) = (0u64, 0u64, None::<u64>);
    for c in text.chars() {
        if let Some(d) = digit(c) {
            part = Some(d);
            continue;
        }
        match c {
            '十' | '拾' | '百' | '佰' | '千' | '仟' => {
                let unit = match c {
                    '十' | '拾' => 10,
                    '百' | '佰' => 100,
                    _ => 1000,
                };
                section += part.unwrap_or(1) * unit;
                part = None;
            }
            '万' | '萬' | '亿' | '億' => {
                let unit = if matches!(c, '万' | '萬') { 10_000 } else { 100_000_000 };
                total = (total + section + part.unwrap_or(0)) * unit;
                section = 0;
                part = None;
            }
            _ => return None,
        }
    }
    Some(total + section + part.unwrap_or(0))
}

/// Bijective base 26: a … z, aa … az, ba.
fn latin(n: u64, base: u8) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut n = n;
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(base + (n % 26) as u8);
        n /= 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn roman(n: u64) -> String {
    if n == 0 || n >= 4000 {
        return n.to_string();
    }
    const TABLE: [(u64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut n = n;
    let mut out = String::new();
    for (value, letters) in TABLE {
        while n >= value {
            out.push_str(letters);
            n -= value;
        }
    }
    out
}

fn read_roman(text: &str) -> Option<u64> {
    let value = |c: char| match c {
        'I' => Some(1),
        'V' => Some(5),
        'X' => Some(10),
        'L' => Some(50),
        'C' => Some(100),
        'D' => Some(500),
        'M' => Some(1000),
        _ => None,
    };
    let values: Vec<u64> = text.chars().map(value).collect::<Option<_>>()?;
    let mut total = 0i64;
    for (i, &v) in values.iter().enumerate() {
        match values.get(i + 1) {
            Some(&next) if next > v => total -= v as i64,
            _ => total += v as i64,
        }
    }
    u64::try_from(total).ok()
}

/// **The first number in `text`**, in Arabic digits or Chinese numerals —
/// `12` and 「第十二章」 both read 12. What `:sort-sections -k number`
/// orders by.
pub fn first_number(text: &str) -> Option<u64> {
    let chars: Vec<char> = text.chars().collect();
    let start = chars
        .iter()
        .position(|&c| Counting::Arabic.spells(c) || Counting::Chinese.spells(c) || Counting::Financial.spells(c))?;
    let system = [Counting::Arabic, Counting::Chinese, Counting::Financial]
        .into_iter()
        .find(|s| s.spells(chars[start]))?;
    let run: String = chars[start..].iter().take_while(|&&c| system.spells(c)).collect();
    system.read(&run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_reads_as_a_book_writes_it() {
        let w = |n| Counting::Chinese.write(n, false);
        assert_eq!(
            [1, 10, 11, 19, 20, 21, 100, 103, 110, 1000, 1005, 10_000, 10_500].map(w),
            ["一", "十", "十一", "十九", "二十", "二十一", "一百", "一百零三", "一百一十", "一千", "一千零五", "一万", "一万零五百"]
        );
        assert_eq!(w(0), "〇");
        assert_eq!(Counting::Chinese.write(10_000, true), "一萬");
    }

    #[test]
    fn financial_forms_say_every_one_and_follow_the_script() {
        let w = |n| Counting::Financial.write(n, false);
        assert_eq!([11, 20, 103, 110].map(w), ["壹拾壹", "贰拾", "壹佰零叁", "壹佰壹拾"]);
        assert_eq!(w(0), "零");
        assert_eq!(Counting::Financial.write(236, true), "貳佰參拾陸");
    }

    #[test]
    fn what_is_written_reads_back() {
        for system in [
            Counting::Arabic,
            Counting::Chinese,
            Counting::Financial,
            Counting::LowerLatin,
            Counting::UpperLatin,
            Counting::LowerRoman,
            Counting::UpperRoman,
        ] {
            for n in [1, 2, 9, 10, 11, 19, 20, 26, 27, 99, 100, 103, 110, 999, 1000, 3999] {
                assert_eq!(system.read(&system.write(n, false)), Some(n), "{system:?} {n}");
                assert_eq!(system.read(&system.write(n, true)), Some(n), "{system:?} {n} traditional");
            }
        }
        assert_eq!(Counting::Chinese.read("一十一"), Some(11));
        assert_eq!(Counting::LowerRoman.read("iiii"), None);
    }

    #[test]
    fn the_first_number_of_a_title() {
        assert_eq!(first_number("第十二章 出山"), Some(12));
        assert_eq!(first_number("Part 7: the end"), Some(7));
        assert_eq!(first_number("序"), None);
    }
}
