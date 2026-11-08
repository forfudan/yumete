//! **Which CJK face a document is set in**: 简体, 繁體 or Japanese, decided
//! once a document from its own text (and its front matter's `lang`), and
//! per run where a `<span lang="…">` says otherwise.
//!
//! A 漢字 shared by all three is drawn differently by each — 骨 and 直 are
//! the famous ones, and every 言 and 辶 differs in detail — so a 繁體 page set
//! in a 简体 face reads as a page in the wrong hand. The faces are the
//! system's own: PingFang SC / TC and Hiragino on macOS, Noto Sans CJK (or
//! Source Han Sans) SC / TC / JP on Linux, YaHei / JhengHei / Yu Gothic on
//! Windows.

/// A CJK writing system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Han {
    #[default]
    Simplified,
    Traditional,
    Japanese,
}

impl Han {
    /// From a language tag: `zh-Hant`, `zh-TW`, `zh_HK`, `ja`, `zh-CN`…
    pub fn from_tag(tag: &str) -> Option<Han> {
        let tag = tag.trim().trim_matches(['"', '\'']).to_ascii_lowercase().replace('_', "-");
        let parts: Vec<&str> = tag.split('-').collect();
        match parts.first().copied()? {
            "ja" | "jp" => Some(Han::Japanese),
            "zh" | "cmn" | "yue" | "lzh" => Some(match parts[1..].iter().any(|p| matches!(*p, "hant" | "tw" | "hk" | "mo")) {
                true => Han::Traditional,
                false => Han::Simplified,
            }),
            _ => None,
        }
    }

    /// The language tag parley is told, for its own fallback and shaping.
    pub fn tag(self) -> &'static str {
        match self {
            Han::Simplified => "zh-Hans",
            Han::Traditional => "zh-Hant",
            Han::Japanese => "ja",
        }
    }

    pub(crate) fn index(self) -> usize {
        self as usize
    }
}

/// Common characters only 简体 writes (their 繁體 forms are below).
const SIMPLIFIED: &str = "们这个说为来时国会对过还学发问开么经长样现进见种动机关从两没点车东给话应产实头书无业让间电认义觉际传写记师设边语读论区识红该钱选饭难换带热视请务员级项总导历术运达队则断题纸观门风飞马鸟鱼龙乐汉华万与码输键简体词组们觉";
/// Common characters only 繁體 writes, 港臺 and 通規 forms both.
const TRADITIONAL: &str = "們這個說為來時國會對過還學發問開麼經長樣現進見種動機關從兩沒點車東給話應產實頭書無業讓間電認義覺際傳寫記師設邊語讀論區識紅該錢選飯難換帶熱視請務員級項總導歷術運達隊則斷題紙觀門風飛馬鳥魚龍樂漢華萬與碼輸鍵簡體詞組爲裏説衆綫";

/// **The writing system of `text`**: Japanese where kana are a real part
/// of it (a ruby's reading is not), else 繁體 where its 繁體-only characters
/// outnumber its 简体-only ones (and there are a few of them: one 過 typed
/// into a page does not turn it), else 简体.
pub fn han_of(text: &str) -> Han {
    let (mut kana, mut han, mut simplified, mut traditional) = (0usize, 0usize, 0usize, 0usize);
    for c in text.chars() {
        match c {
            '\u{3041}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}' => kana += 1,
            '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' => {
                han += 1;
                if SIMPLIFIED.contains(c) {
                    simplified += 1;
                } else if TRADITIONAL.contains(c) {
                    traditional += 1;
                }
            }
            _ => {}
        }
    }
    if kana >= 20 && kana * 5 >= han {
        Han::Japanese
    } else if traditional > simplified && traditional >= 3 {
        Han::Traditional
    } else {
        Han::Simplified
    }
}

/// The font stacks a writing system is set in.
#[derive(Debug, Clone)]
pub(crate) struct Families {
    /// The writing: the UI face for Latin, then the CJK face.
    pub sans: String,
    pub mono: String,
    /// Emphasis: Latin in italics, 漢字 in a 楷體 where there is one.
    pub emphasis: String,
    /// Quotes and dashes between 漢字: the CJK face first.
    pub cjk_first: String,
}

/// The CJK faces of a writing system, best first.
fn cjk(han: Han) -> &'static str {
    match (han, cfg!(target_os = "macos"), cfg!(target_os = "windows")) {
        (Han::Simplified, true, _) => "'PingFang SC', 'Hiragino Sans GB'",
        (Han::Traditional, true, _) => "'PingFang TC', 'PingFang HK', 'Heiti TC', 'PingFang SC'",
        (Han::Japanese, true, _) => "'Hiragino Sans', 'Hiragino Kaku Gothic ProN', 'PingFang SC'",
        (Han::Simplified, _, true) => "'Microsoft YaHei UI', 'Microsoft YaHei'",
        (Han::Traditional, _, true) => "'Microsoft JhengHei UI', 'Microsoft JhengHei', 'Microsoft YaHei'",
        (Han::Japanese, _, true) => "'Yu Gothic UI', 'Yu Gothic', Meiryo, 'Microsoft YaHei'",
        (Han::Simplified, _, _) => "'Noto Sans CJK SC', 'Source Han Sans SC', 'Noto Sans SC', 'WenQuanYi Micro Hei'",
        (Han::Traditional, _, _) => "'Noto Sans CJK TC', 'Source Han Sans TC', 'Noto Sans TC', 'Noto Sans CJK SC', 'WenQuanYi Micro Hei'",
        (Han::Japanese, _, _) => "'Noto Sans CJK JP', 'Source Han Sans', 'Noto Sans JP', 'Noto Sans CJK SC'",
    }
}

/// The 楷體 faces of a writing system; Japanese has no habit of setting
/// emphasis in one, so it keeps its gothic.
fn kai(han: Han) -> &'static str {
    match han {
        Han::Simplified => "'Kaiti SC', STKaiti, KaiTi, 'AR PL UKai CN', 'LXGW WenKai GB', 'LXGW WenKai', ",
        Han::Traditional => "'Kaiti TC', BiauKai, DFKai-SB, 'AR PL UKai TW', 'LXGW WenKai TC', 'Kaiti SC', STKaiti, KaiTi, ",
        Han::Japanese => "",
    }
}

pub(crate) fn families(han: Han) -> Families {
    let (latin, latin_mono, emoji) = if cfg!(target_os = "macos") {
        ("system-ui", "'SF Mono', Menlo, Monaco", "'Apple Color Emoji'")
    } else if cfg!(target_os = "windows") {
        ("'Segoe UI'", "'Cascadia Mono', Consolas", "'Segoe UI Emoji'")
    } else {
        ("system-ui, 'Noto Sans', 'DejaVu Sans'", "'DejaVu Sans Mono', 'Noto Sans Mono', 'Liberation Mono'", "'Noto Color Emoji'")
    };
    let cjk = cjk(han);
    Families {
        sans: format!("{latin}, {cjk}, {emoji}, sans-serif"),
        mono: format!("{latin_mono}, {cjk}, {emoji}, monospace"),
        emphasis: format!("{latin}, {}{cjk}, {emoji}, sans-serif", kai(han)),
        cjk_first: format!("{cjk}, {latin}, sans-serif"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_says_which_hand_it_is_in() {
        assert_eq!(han_of("这是一份用来检查渲染预览的样张，我们说的话。"), Han::Simplified);
        assert_eq!(han_of("常見輸入法測評數據。本文只是呈現輸入法的客觀測評數據，不能用來作爲唯一標準。"), Han::Traditional);
        assert_eq!(han_of("吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。何でも薄暗いじめじめした所でニャーニャー泣いていた事だけは記憶している。"), Han::Japanese);
        // A few readings in ruby do not make a Chinese page Japanese.
        assert_eq!(han_of("这一段说的是<ruby>夢<rt>ゆめ</rt></ruby>，我们会用很多很多的汉字来写这段话，这样才对。"), Han::Simplified);
        assert_eq!(han_of("English only."), Han::Simplified);
    }

    #[test]
    fn language_tags() {
        assert_eq!(Han::from_tag("zh-Hant"), Some(Han::Traditional));
        assert_eq!(Han::from_tag("zh_TW"), Some(Han::Traditional));
        assert_eq!(Han::from_tag("zh-Hant-HK"), Some(Han::Traditional));
        assert_eq!(Han::from_tag("zh-CN"), Some(Han::Simplified));
        assert_eq!(Han::from_tag("zh"), Some(Han::Simplified));
        assert_eq!(Han::from_tag("ja-JP"), Some(Han::Japanese));
        assert_eq!(Han::from_tag("en"), None);
    }

    #[test]
    fn each_hand_has_its_own_faces() {
        let (s, t, j) = (families(Han::Simplified), families(Han::Traditional), families(Han::Japanese));
        assert_ne!(s.sans, t.sans);
        assert_ne!(t.sans, j.sans);
        assert!(s.emphasis.contains("Kai") && !j.emphasis.contains("Kai"));
    }
}
