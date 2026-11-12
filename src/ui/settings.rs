//! Settings screen: Gemini key (Keychain only), models, costs, language and theme.

use iced::widget::{button, column, container, row, text, text_input, toggler};
use iced::{Alignment, Element, Length, Task};

use super::header::{segment, segmented};
use super::input::{fmt_cents, fmt_percent, input_style, parse_shipping};
use super::results::{divider, section_label};
use super::{App, Message};
use crate::domain::{Currency, Money};
use crate::fx::validate_rate;
use crate::i18n::{Key, Lang, t};
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
        .padding([9, 0])
        .width(LABEL_WIDTH)
        .into()
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
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || save_api_key(&key).map(|()| key))
                    .await
                    .map_err(|e| e.to_string())?
            },
            Message::ApiKeySaved,
        )
    }

    pub(super) fn key_saved(&mut self, result: Result<String, String>) {
        match result {
            Ok(key) => {
                self.draft.api_key.clear();
                self.key_notice = None;
                self.set_api_key(Some(key));
            }
            Err(e) => {
                eprintln!("maeve: cannot save API key: {e}");
                self.key_notice = Some(Key::SettingsKeyFailed);
            }
        }
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

    fn gemini_card(&self) -> Element<'_, Message> {
        let lang = self.settings.lang;
        let t_ = tokens(theme::is_dark(&self.theme()));

        let key_input = self
            .text_field(
                t(lang, Key::SettingsApiKeyPlaceholder),
                SettingsField::ApiKey,
                true,
            )
            .secure(true)
            .on_submit(Message::SaveApiKey(self.draft.api_key.clone()));
        let save = button(text(t(lang, Key::SettingsKeySave)).size(13))
            .padding([8, 14])
            .style(theme::button_primary)
            .on_press_maybe(
                (!self.draft.api_key.trim().is_empty())
                    .then(|| Message::SaveApiKey(self.draft.api_key.clone())),
            );
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
        let key_row = row![
            field_label(t(lang, Key::SettingsApiKey)),
            column![
                row![container(key_input).width(CONTROL_WIDTH), save]
                    .spacing(8)
                    .align_y(Alignment::Center),
                status
            ]
            .spacing(4),
        ]
        .spacing(16);

        let card = column![
            section_label(t(lang, Key::SettingsGemini)),
            key_row,
            self.field_row(
                t(lang, Key::SettingsAdvocateModel),
                SettingsField::AdvocateModel,
                None
            ),
            self.field_row(
                t(lang, Key::SettingsJudgeModel),
                SettingsField::JudgeModel,
                None
            ),
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

        let toggle = container(toggler(regimen_on).on_toggle(|on| {
            let mut next = self.settings.clone();
            next.cost.regimen_simplificado = on;
            Message::SettingsChanged(Box::new(next))
        }))
        .padding([9, 0]);
        let regimen_percent = self.labelled(
            t(lang, Key::SettingsRegimenPercent),
            self.text_field("", SettingsField::Regimen, regimen_on),
            None,
            self.draft.is_invalid(SettingsField::Regimen),
        );

        let card = column![
            section_label(t(lang, Key::SettingsCosts)),
            self.field_row(t(lang, Key::SettingsIva), SettingsField::Iva, None),
            self.labelled(t(lang, Key::SettingsRegimen), toggle, None, false),
            regimen_percent,
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

        let _ = app.update(Message::ApiKeySaved(Err("denied".into())));
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
