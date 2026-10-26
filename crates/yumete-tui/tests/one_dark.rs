//! **One Dark has no light half, and says so** (2026-10-25).
//!
//! Warning: **its own test binary.** The theme and the mood in force are
//! process-globals (`theme::choose`, `theme::choose_mood`), so a test that sets
//! them beside two hundred others sets them under them.

use yumete_config::{Config, ThemeConfig};
use yumete_core::command::Mood;
use yumete_core::say;
use yumete_tui::theme::Palette;

#[test]
fn one_dark_stays_dark_and_says_so_when_light_is_asked_for() {
    let config = Config::default();
    let said = yumete_tui::set_theme(&config, Some("one-dark".into()), Some(Mood::Light));
    assert_eq!(said, say!("theme.dark-only", "one-dark"));

    // Asked for nothing about the mood, it reports the mood it is drawn in.
    let said = yumete_tui::set_theme(&config, Some("one-dark".into()), None);
    assert_eq!(said, say!("theme.set", "one-dark", say!("cmd.moods.dark")));

    // In a light mood the page is still One Dark's own ground.
    let theme = ThemeConfig::named("one-dark").expect("one-dark is a theme");
    let light = Palette::of_theme(&theme, false);
    assert_eq!(light.paper(), ratatui::style::Color::Rgb(0x28, 0x2C, 0x34));
    assert_eq!(light.text(), Palette::of_theme(&theme, true).text());

    // **And its code colours and accents, which is what a light mood would
    // actually lose** (2026-10-26 review). The ladder above is the same in
    // both slots, so it passes with or without the dark-only rule; the pinned
    // table and the accents are what come from the mood.
    for name in ["one-dark", "monokai", "dracula"] {
        let theme = ThemeConfig::named(name).expect("a theme by that name");
        let (light, dark) = (Palette::of_theme(&theme, false), Palette::of_theme(&theme, true));
        assert!(light.borrowed(), "{name}");
        assert_eq!(light.pinned(|p| p.keyword), dark.pinned(|p| p.keyword), "{name}");
        assert!(light.pinned(|p| p.keyword).is_some(), "{name}");
        assert_eq!(light.mark(), dark.mark(), "{name}");
        assert_eq!(light.gold(), dark.gold(), "{name}");
    }
}
