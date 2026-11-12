//! Settings screen: Gemini key (Keychain only), models, costs, language and theme.

use iced::widget::{Space, button, column, container, pick_list, row, text, text_input};
use iced::{Alignment, Element, Font, Length, Task};

use super::header::{segment, segmented};
use super::input::{fmt_cents, fmt_percent, input_style, parse_shipping};
use super::results::{divider, section_label};
use super::{App, Message};
use crate::domain::{Currency, Money};
use crate::fx::validate_rate;
use crate::i18n::{Key, Lang, t};
use crate::llm::{GeminiClient, LlmError};
use crate::settings::{Settings, ThemePref, save_api_key};
use crate::theme::{self, tokens};

const MAX_RPM: u32 = 1_000;
const MAX_PERCENT_BP: u32 = 10_000;
const LABEL_WIDTH: f32 = 300.0;
const CONTROL_WIDTH: f32 = 320.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    ApiKey,
    AdvocateModel,
    JudgeModel,
    Rpm,
    Iva,
    Regimen,
    Threshold,
    Fx,
}

/// Raw text of every settings input, so half-typed values survive a rejected parse.
#[derive(Debug, Clone, Default)]
pub struct SettingsDraft {
    api_key: String,
    advocate_model: String,
    judge_model: String,
    rpm: String,
    iva: String,
    regimen: String,
    threshold: String,
    fx: String,
    invalid: Vec<SettingsField>,
}

impl SettingsDraft {
    pub fn from_settings(s: &Settings) -> Self {
        Self {
            api_key: String::new(),
            advocate_model: s.advocate_model.clone(),
            judge_model: s.judge_model.clone(),
            rpm: s.rpm.to_string(),
            iva: fmt_percent(s.cost.iva_bp),
            regimen: fmt_percent(s.cost.regimen_bp),
            threshold: fmt_cents(s.cost.ml_free_shipping_threshold.cents),
            fx: s.fx_manual.map(fmt_rate).unwrap_or_default(),
            invalid: Vec::new(),
        }
    }

    fn slot(&mut self, field: SettingsField) -> &mut String {
        match field {
            SettingsField::ApiKey => &mut self.api_key,
            SettingsField::AdvocateModel => &mut self.advocate_model,
            SettingsField::JudgeModel => &mut self.judge_model,
            SettingsField::Rpm => &mut self.rpm,
            SettingsField::Iva => &mut self.iva,
            SettingsField::Regimen => &mut self.regimen,
            SettingsField::Threshold => &mut self.threshold,
            SettingsField::Fx => &mut self.fx,
        }
    }

    pub fn is_invalid(&self, field: SettingsField) -> bool {
        self.invalid.contains(&field)
    }

    fn mark(&mut self, field: SettingsField, invalid: bool) {
        self.invalid.retain(|f| *f != field);
        if invalid {
            self.invalid.push(field);
        }
    }
}

fn field_label<'a>(label: &str) -> Element<'a, Message> {
    container(text(label.to_owned()).size(14))
        .padding([8, 0])
        .width(LABEL_WIDTH)
        .into()
}

/// Only a key Gemini accepts reaches the Keychain.
async fn verify_and_save(key: String, rpm: u32) -> Result<String, Key> {
    match GeminiClient::new(key.clone(), rpm).verify_key().await {
        Ok(()) => {}
        Err(LlmError::Auth) => return Err(Key::SettingsKeyInvalid),
        Err(e) => {
            eprintln!("maeve: cannot verify API key: {e}");
            return Err(Key::SettingsKeyUnverified);
        }
    }
    tokio::task::spawn_blocking(move || save_api_key(&key).map(|()| key))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r)
        .map_err(|e| {
            eprintln!("maeve: cannot save API key: {e}");
            Key::SettingsKeyFailed
        })
}

fn fmt_rate(rate: f64) -> String {
    rate.to_string().replace('.', ",")
}

/// `"22,5"` is 2250 basis points. Both `,` and `.` are accepted as the decimal
/// separator since a percentage never has a thousands group.
pub(super) fn parse_percent_bp(raw: &str) -> Option<u32> {
    let raw = raw.trim();
    let (int, frac) = raw.split_once([',', '.']).unwrap_or((raw, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if int.is_empty() || !digits(int) || frac.len() > 2 || !digits(frac) {
        return None;
    }
    let whole: u32 = int.parse().ok()?;
    let frac_bp: u32 = match frac.len() {
        0 => 0,
        1 => frac.parse::<u32>().ok()? * 10,
        _ => frac.parse().ok()?,
    };
    let bp = whole.checked_mul(100)?.checked_add(frac_bp)?;
    (bp <= MAX_PERCENT_BP).then_some(bp)
}

/// Empty means automatic.
fn parse_fx(raw: &str) -> Option<Option<f64>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(None);
    }
    let rate = raw.replace(',', ".").parse::<f64>().ok()?;
    validate_rate(rate).ok().map(Some)
}

/// Offered until the API lists what this key can use.
const FALLBACK_MODELS: &[&str] = &["gemini-flash-latest", "gemini-flash-lite-latest"];

/// One entry of the model picker; `id: None` is "Custom…".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    id: Option<String>,
    label: String,
}

impl ModelOption {
    fn model(id: &str) -> Self {
        Self {
            id: Some(id.to_owned()),
            label: id.to_owned(),
        }
    }
}

impl std::fmt::Display for ModelOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

fn model_slot(field: SettingsField) -> Option<usize> {
    match field {
        SettingsField::AdvocateModel => Some(0),
        SettingsField::JudgeModel => Some(1),
        _ => None,
    }
}

fn parse_model(raw: &str) -> Option<String> {
    let model = raw.trim();
    (!model.is_empty() && !model.contains(char::is_whitespace)).then(|| model.to_owned())
}

impl App {
    pub(super) fn apply_settings(&mut self, next: Settings) -> Task<Message> {
        if next == self.settings {
            return Task::none();
        }
        let cost_changed = next.cost != self.settings.cost;
        self.settings = next;
        self.stale |= cost_changed && self.result.is_some();
        self.sync_llm();
        self.save()
    }

    pub(super) fn settings_input(&mut self, field: SettingsField, raw: String) -> Task<Message> {
        *self.draft.slot(field) = raw.clone();
        if field == SettingsField::ApiKey {
            self.key_notice = None;
            return Task::none();
        }
        let mut next = self.settings.clone();
        let valid = match field {
            SettingsField::ApiKey => unreachable!("handled above"),
            SettingsField::AdvocateModel => {
                parse_model(&raw).map(|m| next.advocate_model = m).is_some()
            }
            SettingsField::JudgeModel => parse_model(&raw).map(|m| next.judge_model = m).is_some(),
            SettingsField::Rpm => raw
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|n| (1..=MAX_RPM).contains(n))
                .map(|n| next.rpm = n)
                .is_some(),
            SettingsField::Iva => parse_percent_bp(&raw)
                .map(|bp| next.cost.iva_bp = bp)
                .is_some(),
            SettingsField::Regimen => parse_percent_bp(&raw)
                .map(|bp| next.cost.regimen_bp = bp)
                .is_some(),
            SettingsField::Threshold => parse_shipping(&raw)
                .flatten()
                .map(|cents| {
                    next.cost.ml_free_shipping_threshold = Money {
                        cents,
                        currency: Currency::Uyu,
                    }
                })
                .is_some(),
            SettingsField::Fx => parse_fx(&raw).map(|fx| next.fx_manual = fx).is_some(),
        };
        self.draft.mark(field, !valid);
        if valid {
            self.apply_settings(next)
        } else {
            Task::none()
        }
    }

    pub(super) fn save_key(&mut self, key: String) -> Task<Message> {
        let key = key.trim().to_owned();
        if key.is_empty() {
            self.key_notice = Some(Key::SettingsKeyEmpty);
            return Task::none();
        }
        self.key_notice = None;
        self.key_busy = true;
        let rpm = self.settings.rpm;
        Task::perform(verify_and_save(key, rpm), Message::ApiKeySaved)
    }

    pub(super) fn key_saved(&mut self, result: Result<String, Key>) {
        self.key_busy = false;
        match result {
            Ok(key) => {
                if self.draft.api_key.trim() == key {
                    self.draft.api_key.clear();
                }
                self.key_notice = None;
                self.key_editing = false;
                self.set_api_key(Some(key));
                self.models_requested = false;
            }
            Err(notice) => self.key_notice = Some(notice),
        }
    }

    pub(super) fn load_models(&mut self) -> Task<Message> {
        let Some(client) = self.llm.as_ref().map(|h| std::sync::Arc::clone(&h.client)) else {
            return Task::none();
        };
        if self.models_requested {
            return Task::none();
        }
        self.models_requested = true;
        Task::perform(
            async move { client.list_models().await.map_err(|e| e.to_string()) },
            Message::ModelsLoaded,
        )
    }

    pub(super) fn models_loaded(&mut self, result: Result<Vec<String>, String>) {
        match result {
            Ok(models) if !models.is_empty() => self.models = models,
            Ok(_) => {}
            Err(e) => {
                eprintln!("maeve: cannot list Gemini models: {e}");
                self.models_requested = false;
            }
        }
    }

    pub(super) fn model_picked(
        &mut self,
        field: SettingsField,
        option: ModelOption,
    ) -> Task<Message> {
        let Some(slot) = model_slot(field) else {
            return Task::none();
        };
        match option.id {
            Some(id) => {
                self.custom_model[slot] = false;
                self.settings_input(field, id)
            }
            None => {
                self.custom_model[slot] = true;
                Task::none()
            }
        }
    }

    fn model_row<'a>(&'a self, label: &'a str, field: SettingsField) -> Element<'a, Message> {
        let lang = self.settings.lang;
        let Some(slot) = model_slot(field) else {
            return self.field_row(label, field, None);
        };
        let current = match field {
            SettingsField::AdvocateModel => &self.settings.advocate_model,
            _ => &self.settings.judge_model,
        };
        let known: Vec<String> = if self.models.is_empty() {
            FALLBACK_MODELS.iter().map(|m| (*m).to_owned()).collect()
        } else {
            self.models.clone()
        };
        let custom = self.custom_model[slot] || !known.contains(current);
        let custom_option = ModelOption {
            id: None,
            label: t(lang, Key::ModelCustom).to_owned(),
        };
        let selected = if custom {
            custom_option.clone()
        } else {
            ModelOption::model(current)
        };
        let mut options: Vec<ModelOption> = known.iter().map(|m| ModelOption::model(m)).collect();
        options.push(custom_option);

        let picker = pick_list(options, Some(selected), move |o| {
            Message::ModelPicked(field, o)
        })
        .padding([8, 12])
        .text_size(14)
        .width(Length::Fill)
        .style(theme::select)
        .menu_style(theme::select_menu);
        let mut control = column![picker].spacing(8);
        if custom {
            control = control.push(self.text_field("gemini-…", field, true));
        }
        self.labelled(label, control, None, self.draft.is_invalid(field))
    }

    pub(super) fn edit_key(&mut self, editing: bool) {
        self.key_editing = editing;
        self.key_notice = None;
        self.draft.api_key.clear();
    }

    fn text_field<'a>(
        &'a self,
        placeholder: &str,
        field: SettingsField,
        enabled: bool,
    ) -> text_input::TextInput<'a, Message> {
        let value = match field {
            SettingsField::ApiKey => &self.draft.api_key,
            SettingsField::AdvocateModel => &self.draft.advocate_model,
            SettingsField::JudgeModel => &self.draft.judge_model,
            SettingsField::Rpm => &self.draft.rpm,
            SettingsField::Iva => &self.draft.iva,
            SettingsField::Regimen => &self.draft.regimen,
            SettingsField::Threshold => &self.draft.threshold,
            SettingsField::Fx => &self.draft.fx,
        };
        text_input(placeholder, value)
            .on_input_maybe(enabled.then_some(move |s: String| Message::SettingsInput(field, s)))
            .padding([8, 12])
            .size(14)
            .style(input_style(self.draft.is_invalid(field)))
            .width(Length::Fill)
    }

    fn labelled<'a>(
        &self,
        label: &'a str,
        control: impl Into<Element<'a, Message>>,
        hint: Option<String>,
        invalid: bool,
    ) -> Element<'a, Message> {
        let lang = self.settings.lang;
        let t_ = tokens(theme::is_dark(&self.theme()));
        let mut right = column![container(control).width(CONTROL_WIDTH)].spacing(4);
        if invalid {
            right = right.push(
                text(t(lang, Key::InvalidValue))
                    .size(12)
                    .color(t_.destructive),
            );
        } else if let Some(hint) = hint {
            right = right.push(text(hint).size(12).style(theme::text_muted));
        }
        row![field_label(label), right]
            .spacing(16)
            .align_y(Alignment::Start)
            .into()
    }

    fn field_row<'a>(
        &'a self,
        label: &'a str,
        field: SettingsField,
        hint: Option<String>,
    ) -> Element<'a, Message> {
        self.labelled(
            label,
            self.text_field("", field, true),
            hint,
            self.draft.is_invalid(field),
        )
    }

    fn key_configured(&self, key: &str) -> Element<'_, Message> {
        let lang = self.settings.lang;
        let t_ = tokens(theme::is_dark(&self.theme()));
        let tail: String = key
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let summary = container(
            row![
                text("✓").size(14).color(t_.chart[0]),
                text(t(lang, Key::SettingsKeyConfigured)).size(14),
                Space::new().width(Length::Fill),
                text(format!("••••{tail}"))
                    .size(14)
                    .font(Font::MONOSPACE)
                    .style(theme::text_muted),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([8, 12])
        .width(CONTROL_WIDTH)
        .style(theme::list_item);
        let change = button(text(t(lang, Key::SettingsKeyChange)).size(13))
            .padding([8, 16])
            .style(theme::button_secondary)
            .on_press(Message::EditApiKey(true));
        let status = text(t(lang, Key::SettingsKeyStored))
            .size(12)
            .style(theme::text_muted);
        column![
            row![summary, change].spacing(8).align_y(Alignment::Center),
            status
        ]
        .spacing(4)
        .into()
    }

    fn key_form(&self) -> Element<'_, Message> {
        let lang = self.settings.lang;
        let t_ = tokens(theme::is_dark(&self.theme()));

        let key_input = self
            .text_field(
                t(lang, Key::SettingsApiKeyPlaceholder),
                SettingsField::ApiKey,
                !self.key_busy,
            )
            .secure(true)
            .on_submit_maybe(
                (!self.key_busy).then(|| Message::SaveApiKey(self.draft.api_key.clone())),
            );
        let save_label = if self.key_busy {
            Key::SettingsKeyVerifying
        } else {
            Key::SettingsKeySave
        };
        let save = button(text(t(lang, save_label)).size(13))
            .padding([8, 16])
            .style(theme::button_primary)
            .on_press_maybe(
                (!self.key_busy && !self.draft.api_key.trim().is_empty())
                    .then(|| Message::SaveApiKey(self.draft.api_key.clone())),
            );
        let mut controls = row![container(key_input).width(CONTROL_WIDTH), save]
            .spacing(8)
            .align_y(Alignment::Center);
        if self.api_key.is_some() {
            controls = controls.push(
                button(text(t(lang, Key::Cancel)).size(13))
                    .padding([8, 16])
                    .style(theme::button_ghost)
                    .on_press_maybe((!self.key_busy).then_some(Message::EditApiKey(false))),
            );
        }
        let status: Element<'_, Message> = match self.key_notice {
            Some(notice) => text(t(lang, notice)).size(12).color(t_.destructive).into(),
            None => {
                let key = if self.api_key.is_some() {
                    Key::SettingsKeyStored
                } else {
                    Key::SettingsKeyMissing
                };
                text(t(lang, key)).size(12).style(theme::text_muted).into()
            }
        };
        column![controls, status].spacing(4).into()
    }

    fn gemini_card(&self) -> Element<'_, Message> {
        let lang = self.settings.lang;
        let key_control = match &self.api_key {
            Some(key) if !self.key_editing => self.key_configured(key),
            _ => self.key_form(),
        };
        let key_row = row![field_label(t(lang, Key::SettingsApiKey)), key_control].spacing(16);

        let card = column![
            section_label(t(lang, Key::SettingsGemini)),
            key_row,
            self.model_row(
                t(lang, Key::SettingsAdvocateModel),
                SettingsField::AdvocateModel,
            ),
            self.model_row(t(lang, Key::SettingsJudgeModel), SettingsField::JudgeModel),
            self.field_row(t(lang, Key::SettingsRpm), SettingsField::Rpm, None),
        ]
        .spacing(16);
        container(card)
            .padding(24)
            .width(Length::Fill)
            .style(theme::card)
            .into()
    }

    fn costs_card(&self) -> Element<'_, Message> {
        let lang = self.settings.lang;
        let regimen_on = self.settings.cost.regimen_simplificado;

        let pick = |regimen: bool| {
            let mut next = self.settings.clone();
            next.cost.regimen_simplificado = regimen;
            Message::SettingsChanged(Box::new(next))
        };
        let tax_kind = segmented([
            segment(t(lang, Key::TaxIva).into(), !regimen_on, pick(false)),
            segment(t(lang, Key::SettingsRegimen).into(), regimen_on, pick(true)),
        ]);
        let tax_percent = if regimen_on {
            self.field_row(
                t(lang, Key::SettingsRegimenPercent),
                SettingsField::Regimen,
                None,
            )
        } else {
            self.field_row(t(lang, Key::SettingsIva), SettingsField::Iva, None)
        };

        let card = column![
            section_label(t(lang, Key::SettingsCosts)),
            self.labelled(
                t(lang, Key::SettingsTemuTax),
                row![tax_kind],
                Some(t(lang, Key::SettingsTemuTaxHint).to_owned()),
                false
            ),
            tax_percent,
            self.field_row(
                t(lang, Key::SettingsMlThreshold),
                SettingsField::Threshold,
                None
            ),
            self.field_row(
                t(lang, Key::SettingsFx),
                SettingsField::Fx,
                Some(t(lang, Key::SettingsFxHint).to_owned())
            ),
        ]
        .spacing(16);
        container(card)
            .padding(24)
            .width(Length::Fill)
            .style(theme::card)
            .into()
    }

    fn appearance_card(&self) -> Element<'_, Message> {
        let lang = self.settings.lang;
        let theme_pref = self.settings.theme;

        let langs = segmented([
            segment(
                t(lang, Key::LangEs).into(),
                lang == Lang::Es,
                Message::SetLang(Lang::Es),
            ),
            segment(
                t(lang, Key::LangEn).into(),
                lang == Lang::En,
                Message::SetLang(Lang::En),
            ),
        ]);
        let themes = segmented(
            [
                (Key::ThemeSystem, ThemePref::System),
                (Key::ThemeLight, ThemePref::Light),
                (Key::ThemeDark, ThemePref::Dark),
            ]
            .map(|(key, pref)| {
                segment(
                    t(lang, key).into(),
                    theme_pref == pref,
                    Message::SetThemePref(pref),
                )
            }),
        );

        let card = column![
            section_label(t(lang, Key::SettingsAppearance)),
            row![
                text(t(lang, Key::SettingsLanguage))
                    .size(14)
                    .width(LABEL_WIDTH),
                langs
            ]
            .spacing(16)
            .align_y(Alignment::Center),
            divider(),
            row![
                text(t(lang, Key::SettingsTheme))
                    .size(14)
                    .width(LABEL_WIDTH),
                themes
            ]
            .spacing(16)
            .align_y(Alignment::Center),
        ]
        .spacing(16);
        container(card)
            .padding(24)
            .width(Length::Fill)
            .style(theme::card)
            .into()
    }

    pub(super) fn settings_view(&self) -> Element<'_, Message> {
        let lang = self.settings.lang;
        column![
            text(t(lang, Key::NavSettings)).size(22),
            self.gemini_card(),
            self.costs_card(),
            self.appearance_card(),
        ]
        .spacing(16)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ui::snap::DevEnv;

    fn app() -> App {
        App::new(Settings::default(), None, DevEnv::default())
    }

    fn demo_app() -> App {
        let dev = DevEnv {
            demo: true,
            ..DevEnv::default()
        };
        App::new(Settings::default(), None, dev)
    }

    fn client(app: &App) -> Arc<crate::llm::GeminiClient> {
        Arc::clone(&app.llm.as_ref().unwrap().client)
    }

    #[test]
    fn percent_accepts_comma_or_dot_decimals() {
        assert_eq!(parse_percent_bp("22"), Some(2200));
        assert_eq!(parse_percent_bp("22,5"), Some(2250));
        assert_eq!(parse_percent_bp("22.5"), Some(2250));
        assert_eq!(parse_percent_bp(" 22,05 "), Some(2205));
        assert_eq!(parse_percent_bp("0"), Some(0));
        assert_eq!(parse_percent_bp("100"), Some(10_000));
        assert_eq!(parse_percent_bp("22,"), Some(2200));
    }

    #[test]
    fn percent_rejects_malformed_values() {
        for bad in [
            "", "abc", "-1", "22,5x", ",5", "22,555", "1,2,3", "100,01", "101", "2 2",
        ] {
            assert_eq!(parse_percent_bp(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn iva_input_stores_basis_points() {
        let mut app = app();
        let _ = app.update(Message::SettingsInput(SettingsField::Iva, "22,5".into()));
        assert_eq!(app.settings.cost.iva_bp, 2250);
        assert!(!app.draft.is_invalid(SettingsField::Iva));
    }

    #[test]
    fn invalid_input_keeps_previous_value_and_flags_the_field() {
        let mut app = app();
        let _ = app.update(Message::SettingsInput(SettingsField::Iva, "22,5".into()));
        let _ = app.update(Message::SettingsInput(SettingsField::Iva, "22,5x".into()));
        assert_eq!(app.settings.cost.iva_bp, 2250);
        assert!(app.draft.is_invalid(SettingsField::Iva));
        let _ = app.update(Message::SettingsInput(SettingsField::Iva, "23".into()));
        assert_eq!(app.settings.cost.iva_bp, 2300);
        assert!(!app.draft.is_invalid(SettingsField::Iva));
    }

    #[test]
    fn rpm_change_rebuilds_the_client() {
        let mut app = app();
        app.set_api_key(Some("k".into()));
        let before = client(&app);
        let _ = app.update(Message::SettingsInput(SettingsField::Rpm, "5".into()));
        assert_eq!(app.settings.rpm, 5);
        assert!(!Arc::ptr_eq(&before, &client(&app)));
    }

    #[test]
    fn rpm_must_be_positive_and_bounded() {
        let mut app = app();
        for bad in ["0", "abc", "", "1001", "-3"] {
            let _ = app.update(Message::SettingsInput(SettingsField::Rpm, bad.into()));
            assert_eq!(app.settings.rpm, 10, "{bad:?}");
            assert!(app.draft.is_invalid(SettingsField::Rpm));
        }
    }

    #[test]
    fn unrelated_change_keeps_the_client() {
        let mut app = app();
        app.set_api_key(Some("k".into()));
        let before = client(&app);
        let _ = app.update(Message::SettingsInput(SettingsField::Iva, "20".into()));
        assert!(Arc::ptr_eq(&before, &client(&app)));
    }

    #[test]
    fn threshold_and_models_update() {
        let mut app = app();
        let _ = app.update(Message::SettingsInput(
            SettingsField::Threshold,
            "1.500".into(),
        ));
        assert_eq!(app.settings.cost.ml_free_shipping_threshold.cents, 150_000);
        let _ = app.update(Message::SettingsInput(
            SettingsField::JudgeModel,
            " gemini-pro-latest ".into(),
        ));
        assert_eq!(app.settings.judge_model, "gemini-pro-latest");
        let _ = app.update(Message::SettingsInput(
            SettingsField::AdvocateModel,
            "".into(),
        ));
        assert_eq!(app.settings.advocate_model, "gemini-flash-latest");
        assert!(app.draft.is_invalid(SettingsField::AdvocateModel));
    }

    #[test]
    fn manual_rate_is_optional_and_validated() {
        let mut app = app();
        let _ = app.update(Message::SettingsInput(SettingsField::Fx, "40,5".into()));
        assert_eq!(app.settings.fx_manual, Some(40.5));
        let _ = app.update(Message::SettingsInput(SettingsField::Fx, "0".into()));
        assert_eq!(app.settings.fx_manual, Some(40.5));
        assert!(app.draft.is_invalid(SettingsField::Fx));
        let _ = app.update(Message::SettingsInput(SettingsField::Fx, "".into()));
        assert_eq!(app.settings.fx_manual, None);
        assert!(!app.draft.is_invalid(SettingsField::Fx));
    }

    #[test]
    fn regimen_toggle_goes_through_settings_changed() {
        let mut app = app();
        let mut next = app.settings.clone();
        next.cost.regimen_simplificado = true;
        let _ = app.update(Message::SettingsChanged(Box::new(next)));
        assert!(app.settings.cost.regimen_simplificado);
    }

    #[test]
    fn cost_change_marks_a_result_stale() {
        let mut app = demo_app();
        assert!(!app.stale);
        let _ = app.update(Message::SettingsInput(SettingsField::Fx, "41".into()));
        assert!(!app.stale);
        let _ = app.update(Message::SettingsInput(SettingsField::Iva, "10".into()));
        assert!(app.stale);
    }

    #[tokio::test]
    async fn settings_changes_are_saved() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            crate::store::Store::open(&dir.path().join("t.db"))
                .await
                .unwrap(),
        );
        let mut app = App::new(Settings::default(), Some(store), DevEnv::default());
        let task = app.update(Message::SettingsInput(SettingsField::Iva, "20".into()));
        assert_eq!(task.units(), 1);
        let task = app.update(Message::SettingsInput(SettingsField::Iva, "20".into()));
        assert_eq!(task.units(), 0);
    }

    #[test]
    fn empty_key_is_rejected_without_a_task() {
        let mut app = app();
        let task = app.update(Message::SaveApiKey("   ".into()));
        assert_eq!(task.units(), 0);
        assert_eq!(app.key_notice, Some(Key::SettingsKeyEmpty));
    }

    #[test]
    fn key_is_only_used_once_the_keychain_accepted_it() {
        let mut app = app();
        let task = app.update(Message::SaveApiKey(" secret ".into()));
        assert_eq!(task.units(), 1);
        assert!(app.llm.is_none());

        let _ = app.update(Message::ApiKeySaved(Err(Key::SettingsKeyFailed)));
        assert!(app.llm.is_none());
        assert_eq!(app.key_notice, Some(Key::SettingsKeyFailed));

        let _ = app.update(Message::SettingsInput(
            SettingsField::ApiKey,
            "secret".into(),
        ));
        assert_eq!(app.key_notice, None);
        let _ = app.update(Message::ApiKeySaved(Ok("secret".into())));
        assert_eq!(app.llm.as_ref().unwrap().key, "secret");
        assert!(app.draft.api_key.is_empty());
    }

    #[test]
    fn changing_a_configured_key_returns_to_the_summary_once_saved() {
        let mut app = app();
        app.set_api_key(Some("old".into()));
        let _ = app.update(Message::EditApiKey(true));
        assert!(app.key_editing);

        let _ = app.update(Message::SaveApiKey("new".into()));
        assert!(app.key_busy);
        let _ = app.update(Message::ApiKeySaved(Err(Key::SettingsKeyInvalid)));
        assert!(!app.key_busy && app.key_editing);
        assert_eq!(app.llm.as_ref().unwrap().key, "old");

        let _ = app.update(Message::ApiKeySaved(Ok("new".into())));
        assert!(!app.key_editing);
        assert_eq!(app.llm.as_ref().unwrap().key, "new");
    }

    #[test]
    fn picking_a_model_saves_it_and_custom_keeps_the_value() {
        let mut app = app();
        let _ = app.update(Message::ModelPicked(
            SettingsField::JudgeModel,
            ModelOption::model("gemini-flash-lite-latest"),
        ));
        assert_eq!(app.settings.judge_model, "gemini-flash-lite-latest");
        assert!(!app.custom_model[1]);

        let custom = ModelOption {
            id: None,
            label: "Custom…".into(),
        };
        let _ = app.update(Message::ModelPicked(SettingsField::JudgeModel, custom));
        assert!(app.custom_model[1]);
        assert_eq!(app.settings.judge_model, "gemini-flash-lite-latest");
    }

    #[test]
    fn listed_models_replace_the_fallback_and_errors_allow_a_retry() {
        let mut app = app();
        app.models_requested = true;
        let _ = app.update(Message::ModelsLoaded(Err("offline".into())));
        assert!(!app.models_requested && app.models.is_empty());
        let _ = app.update(Message::ModelsLoaded(Ok(vec!["gemini-x".into()])));
        assert_eq!(app.models, ["gemini-x"]);
    }

    #[test]
    fn key_never_enters_the_settings_json() {
        let mut app = app();
        let _ = app.update(Message::SettingsInput(
            SettingsField::ApiKey,
            "super-secret".into(),
        ));
        assert!(
            !serde_json::to_string(&app.settings)
                .unwrap()
                .contains("super-secret")
        );
    }

    #[test]
    fn draft_mirrors_loaded_settings() {
        let mut s = Settings::default();
        s.cost.iva_bp = 2250;
        s.fx_manual = Some(40.5);
        let draft = SettingsDraft::from_settings(&s);
        assert_eq!(draft.iva, "22,5");
        assert_eq!(draft.fx, "40,5");
        assert_eq!(draft.threshold, "1000");
        assert_eq!(draft.rpm, "10");
    }

    #[test]
    fn booted_settings_refresh_the_draft() {
        let mut app = app();
        let s = Settings {
            rpm: 7,
            ..Settings::default()
        };
        let _ = app.update(Message::Booted(None, Box::new(s)));
        assert_eq!(app.draft.rpm, "7");
    }
}
