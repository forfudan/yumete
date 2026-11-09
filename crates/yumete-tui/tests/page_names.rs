//! **One answer for a page's name**: `[export] page`, read by
//! yumete-config, and `:export page=…`, read by yumete-core, which cannot see
//! each other. This crate sees both.

#[test]
fn the_setting_and_the_export_word_read_the_same_pages() {
    for name in [
        "a4", "A5", "a6", "B5", "b6", "letter", "16k", "16開", "16开", "32k", "32開", "32开", "d32k", "大32開", "大32开",
        "130x184", "137 x 195", "140*203", "148×210", "0x210", "148", "quarto", "",
    ] {
        let config = yumete_config::parse_page(name);
        let core = yumete_core::export::Paper::named(name).map(|p| (p.width, p.height));
        assert_eq!(config, core, "{name:?}");
    }
}
