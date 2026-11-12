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
    StatusScraping,
    StatusReady,
    StatusFailed,
    StatusExtracting,
    Retry,
    ShippingPlaceholder,
    CostProduct,
    CostShipping,
    CostIva,
    CostRegimen,
    CostTotal,
    ShippingToConfirm,
    ProgressScraping,
    ProgressAdvocates,
    ProgressWaiting,
    ProgressJudging,
    NoApiKey,
    NotEnoughProducts,
    ApiKeyRejected,
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
        Key::StatusScraping,
        Key::StatusReady,
        Key::StatusFailed,
        Key::StatusExtracting,
        Key::Retry,
        Key::ShippingPlaceholder,
        Key::CostProduct,
        Key::CostShipping,
        Key::CostIva,
        Key::CostRegimen,
        Key::CostTotal,
        Key::ShippingToConfirm,
        Key::ProgressScraping,
        Key::ProgressAdvocates,
        Key::ProgressWaiting,
        Key::ProgressJudging,
        Key::NoApiKey,
        Key::NotEnoughProducts,
        Key::ApiKeyRejected,
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
        (StatusScraping, Es) => "Leyendo…",
        (StatusScraping, En) => "Reading…",
        (StatusReady, Es) => "Listo",
        (StatusReady, En) => "Ready",
        (StatusFailed, Es) => "Falló",
        (StatusFailed, En) => "Failed",
        (StatusExtracting, Es) => "Extrayendo…",
        (StatusExtracting, En) => "Extracting…",
        (Retry, Es) => "Reintentar",
        (Retry, En) => "Retry",
        (ShippingPlaceholder, Es) => "Envío ($)",
        (ShippingPlaceholder, En) => "Shipping ($)",
        (CostProduct, Es) => "Producto",
        (CostProduct, En) => "Product",
        (CostShipping, Es) => "Envío",
        (CostShipping, En) => "Shipping",
        (CostIva, Es) => "IVA",
        (CostIva, En) => "VAT",
        (CostRegimen, Es) => "Régimen",
        (CostRegimen, En) => "Regime",
        (CostTotal, Es) => "Total",
        (CostTotal, En) => "Total",
        (ShippingToConfirm, Es) => "a confirmar",
        (ShippingToConfirm, En) => "to confirm",
        (ProgressScraping, Es) => "Leyendo productos…",
        (ProgressScraping, En) => "Reading products…",
        (ProgressAdvocates, Es) => "Defensores",
        (ProgressAdvocates, En) => "Advocates",
        (ProgressWaiting, Es) => "esperando límite de Gemini",
        (ProgressWaiting, En) => "waiting for Gemini rate limit",
        (ProgressJudging, Es) => "El juez está decidiendo…",
        (ProgressJudging, En) => "The judge is deciding…",
        (NoApiKey, Es) => "Configurá tu clave de Gemini en Ajustes para comparar.",
        (NoApiKey, En) => "Set your Gemini key in Settings to compare.",
        (NotEnoughProducts, Es) => "Hacen falta al menos 2 productos legibles para comparar.",
        (NotEnoughProducts, En) => "At least 2 readable products are needed to compare.",
        (ApiKeyRejected, Es) => "Gemini rechazó tu clave. Revisala en Ajustes.",
        (ApiKeyRejected, En) => "Gemini rejected your key. Check it in Settings.",
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
