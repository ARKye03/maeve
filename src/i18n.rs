use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Lang {
    #[default]
    Es,
    En,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    AppName,
    ThemeSystem,
    ThemeLight,
    ThemeDark,
    LangEs,
    LangEn,
    UrlPlaceholder,
    AddUrl,
    RemoveUrl,
    PresetBestOverall,
    PresetBestValue,
    PresetCheapest,
    PresetCustom,
    CustomCriteriaPlaceholder,
    PurposePlaceholder,
    Compare,
    NavHistory,
    NavSettings,
}

impl Key {
    pub const ALL: &'static [Key] = &[
        Key::AppName,
        Key::ThemeSystem,
        Key::ThemeLight,
        Key::ThemeDark,
        Key::LangEs,
        Key::LangEn,
        Key::UrlPlaceholder,
        Key::AddUrl,
        Key::RemoveUrl,
        Key::PresetBestOverall,
        Key::PresetBestValue,
        Key::PresetCheapest,
        Key::PresetCustom,
        Key::CustomCriteriaPlaceholder,
        Key::PurposePlaceholder,
        Key::Compare,
        Key::NavHistory,
        Key::NavSettings,
    ];
}

pub fn t(lang: Lang, key: Key) -> &'static str {
    use Key::*;
    use Lang::*;
    match (key, lang) {
        (AppName, _) => "Maeve",
        (ThemeSystem, Es) => "Sistema",
        (ThemeSystem, En) => "System",
        (ThemeLight, Es) => "Claro",
        (ThemeLight, En) => "Light",
        (ThemeDark, Es) => "Oscuro",
        (ThemeDark, En) => "Dark",
        (LangEs, _) => "ES",
        (LangEn, _) => "EN",
        (UrlPlaceholder, Es) => "Pegá el link de un producto",
        (UrlPlaceholder, En) => "Paste a product link",
        (AddUrl, Es) => "+ Agregar link",
        (AddUrl, En) => "+ Add link",
        (RemoveUrl, Es) => "Quitar",
        (RemoveUrl, En) => "Remove",
        (PresetBestOverall, Es) => "Mejor en general",
        (PresetBestOverall, En) => "Best overall",
        (PresetBestValue, Es) => "Mejor precio-calidad",
        (PresetBestValue, En) => "Best quality-price",
        (PresetCheapest, Es) => "Más barato decente",
        (PresetCheapest, En) => "Cheapest decent",
        (PresetCustom, Es) => "Personalizado",
        (PresetCustom, En) => "Custom",
        (CustomCriteriaPlaceholder, Es) => "Tus criterios, en texto libre",
        (CustomCriteriaPlaceholder, En) => "Your criteria, free text",
        (PurposePlaceholder, Es) => "Para qué lo vas a usar",
        (PurposePlaceholder, En) => "What you will use it for",
        (Compare, Es) => "Comparar",
        (Compare, En) => "Compare",
        (NavHistory, Es) => "Historial",
        (NavHistory, En) => "History",
        (NavSettings, Es) => "Ajustes",
        (NavSettings, En) => "Settings",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_text_in_both_languages() {
        for &k in Key::ALL {
            assert!(!t(Lang::Es, k).is_empty(), "{k:?} es");
            assert!(!t(Lang::En, k).is_empty(), "{k:?} en");
        }
    }

    #[test]
    fn translated_keys_differ_between_languages() {
        assert_ne!(t(Lang::Es, Key::Compare), t(Lang::En, Key::Compare));
        assert_eq!(t(Lang::Es, Key::AppName), t(Lang::En, Key::AppName));
    }

    #[test]
    fn default_is_spanish() {
        assert_eq!(Lang::default(), Lang::Es);
    }
}
