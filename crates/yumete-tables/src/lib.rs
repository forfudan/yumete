//! The 宇浩 tables the `yumete` binary carries: 宇浩拼音 with its language
//! layer and sentence model, and the full 靈明 and 星陳.
//!
//! Each file is kept zstd-compressed under its manifest name
//! (`data/pinyin.yflb`, `schemes/lingming.ytab`, …) and unpacked when it is
//! asked for. Where they come from, and why that is fixed, is in `build.rs`.

mod built {
    include!(concat!(env!("OUT_DIR"), "/tables.rs"));
}

/// The files this binary carries, by manifest name, still compressed.
pub fn names() -> impl Iterator<Item = &'static str> {
    built::FILES.iter().map(|(name, _)| *name)
}

/// Whether this binary carries `name`.
pub fn has(name: &str) -> bool {
    built::FILES.iter().any(|(n, _)| *n == name)
}

/// The bytes of `name`, unpacked; `None` when this binary does not carry it.
pub fn file(name: &str) -> Option<Vec<u8>> {
    let (_, packed) = built::FILES.iter().find(|(n, _)| *n == name)?;
    zstd::decode_all(*packed).ok()
}

/// Which Yume release the tables came from, as `2026-10-29 16:31`.
pub const VERSION: Option<&str> = built::VERSION;
