//! Dev-only screenshot harness, driven by `MAEVE_*` environment variables.

use std::path::{Path, PathBuf};
use std::time::Duration;

use iced::window::Screenshot;

use super::Screen;
use crate::i18n::Lang;
use crate::settings::{Settings, ThemePref};

pub const SETTLE: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DevEnv {
    pub demo: bool,
    pub theme: Option<ThemePref>,
    pub lang: Option<Lang>,
    pub screen: Option<Screen>,
    pub scroll: Option<f32>,
    pub debate: bool,
    pub failed: bool,
    pub snapshot: Option<PathBuf>,
    pub gemini_key: Option<String>,
}

impl DevEnv {
    pub fn from_env() -> Self {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            demo: get("MAEVE_DEMO").is_some_and(|v| !matches!(v.as_str(), "" | "0")),
            theme: get("MAEVE_THEME").and_then(|v| match v.as_str() {
                "light" => Some(ThemePref::Light),
                "dark" => Some(ThemePref::Dark),
                _ => None,
            }),
            lang: get("MAEVE_LANG").and_then(|v| match v.as_str() {
                "es" => Some(Lang::Es),
                "en" => Some(Lang::En),
                _ => None,
            }),
            screen: get("MAEVE_SCREEN").and_then(|v| match v.as_str() {
                "compare" => Some(Screen::Compare),
                "history" => Some(Screen::History),
                "settings" => Some(Screen::Settings),
                _ => None,
            }),
            scroll: get("MAEVE_SCROLL")
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| (0.0..=1.0).contains(v)),
            debate: get("MAEVE_DEBATE").is_some_and(|v| !matches!(v.as_str(), "" | "0")),
            failed: get("MAEVE_FAILED").is_some_and(|v| !matches!(v.as_str(), "" | "0")),
            snapshot: get("MAEVE_SNAPSHOT")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            gemini_key: if cfg!(debug_assertions) {
                get("MAEVE_GEMINI_KEY").filter(|v| !v.is_empty())
            } else {
                None
            },
        }
    }

    pub fn apply(&self, settings: &mut Settings) {
        if let Some(theme) = self.theme {
            settings.theme = theme;
        }
        if let Some(lang) = self.lang {
            settings.lang = lang;
        }
    }
}

pub fn save_png(path: &Path, shot: &Screenshot) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    image::save_buffer(
        path,
        &shot.rgba,
        shot.size.width,
        shot.size.height,
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> DevEnv {
        DevEnv::from_lookup(|k| {
            pairs
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| (*v).to_owned())
        })
    }

    #[test]
    fn empty_env_is_inert() {
        assert_eq!(env(&[]), DevEnv::default());
    }

    #[test]
    fn parses_overrides() {
        let e = env(&[
            ("MAEVE_DEMO", "1"),
            ("MAEVE_THEME", "dark"),
            ("MAEVE_LANG", "en"),
            ("MAEVE_SCREEN", "history"),
            ("MAEVE_SCROLL", "0.5"),
            ("MAEVE_DEBATE", "1"),
            ("MAEVE_FAILED", "1"),
            ("MAEVE_SNAPSHOT", "target/snaps/x.png"),
        ]);
        assert!(e.demo);
        assert_eq!(e.theme, Some(ThemePref::Dark));
        assert_eq!(e.lang, Some(Lang::En));
        assert_eq!(e.screen, Some(Screen::History));
        assert_eq!(e.scroll, Some(0.5));
        assert!(e.debate);
        assert!(e.failed);
        assert_eq!(e.snapshot, Some(PathBuf::from("target/snaps/x.png")));
    }

    #[test]
    fn unknown_values_are_ignored() {
        let e = env(&[
            ("MAEVE_THEME", "sepia"),
            ("MAEVE_DEMO", "0"),
            ("MAEVE_SCROLL", "2"),
        ]);
        assert_eq!(e, DevEnv::default());
    }

    #[test]
    fn saves_png() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out/x.png");
        let shot = Screenshot::new(vec![255u8; 2 * 2 * 4], iced::Size::new(2, 2), 1.0);
        save_png(&path, &shot).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
    }
}
