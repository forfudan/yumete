//! **The session keeps only the theme the config would not give** (2026-10-29).
//!
//! Warning: **its own test binary.** The theme and the mood in force are
//! process-globals (`theme::choose`, `theme::choose_mood`), so a test that sets
//! them beside two hundred others sets them under them.

use yumete_config::{Config, Mode, ThemeConfig};
use yumete_core::Editor;
use yumete_tui::{settings::view_to_keep, theme};

#[test]
fn a_chosen_theme_the_config_gives_too_is_not_kept() {
    let mut config = Config::default();
    let kept = |config: &Config| -> Vec<String> {
        view_to_keep(&Editor::new(), config).into_iter().filter(|l| l.starts_with("theme")).collect()
    };

    theme::choose_mood(true);
    assert_eq!(kept(&config), ["theme-mode dark"]);
    config.theme.mode = Mode::Dark;
    assert!(kept(&config).is_empty(), "the config says dark already");

    theme::choose(ThemeConfig::named("one-dark").expect("a theme by that name"));
    assert_eq!(kept(&config), ["theme one-dark"]);
    config.theme.name = "one-dark".into();
    assert!(kept(&config).is_empty(), "the config names it already");
}
