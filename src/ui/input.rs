//! Input card and the compare flow (scrape, rate resolution, pipeline events).

use std::sync::Arc;
use std::time::Duration;

use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Alignment, Background, Border, Color, Element, Length, Task, Theme};

use super::{App, ItemState, ItemStatus, LlmHandle, Message, RunError, RunState};
use crate::domain::{
    CostBreakdown, Currency, Money, Product, ShippingCost, TaxKind, format_money, landed_cost,
};
use crate::fx::{DEFAULT_BASE_URL, RatePick, fetch_rate, pick_rate, validate_fx};
use crate::i18n::{Key, Lang, t};
use crate::llm::prompts::Preset;
use crate::llm::{GeminiClient, LlmError};
use crate::pipeline::{CompareInput, ItemInput, PipelineError, PipelineEvent, run_comparison};
use crate::scrape::ScrapeErrorKind;
use crate::theme::{self, tokens};

/// Name given to the free-text preset so history has a meaningful label.
const AD_HOC_NAME: &str = "custom";
const MAX_PRESET_NAME: usize = 40;

pub(super) fn ad_hoc_preset(criteria: String) -> Preset {
    Preset::Custom {
        name: AD_HOC_NAME.into(),
        criteria,
    }
}

fn is_ad_hoc(p: &Preset) -> bool {
    matches!(p, Preset::Custom { name, .. } if name == AD_HOC_NAME)
}

/// Uruguayan amount: `.` only groups thousands (`1.500`), `,` is the decimal
/// separator. Outer `None` is invalid input, inner `None` is empty input.
pub(super) fn parse_shipping(raw: &str) -> Option<Option<i64>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(None);
    }
    let (int, frac) = raw.split_once(',').unwrap_or((raw, ""));
    let mut groups = int.split('.');
    let first = groups.next()?;
    let rest: Vec<&str> = groups.collect();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let grouped_ok = if rest.is_empty() {
        digits(first)
    } else {
        digits(first) && first.len() <= 3 && rest.iter().all(|g| g.len() == 3 && digits(g))
    };
    let frac_ok = frac.len() <= 2 && frac.bytes().all(|b| b.is_ascii_digit());
    if !grouped_ok || !frac_ok {
        return None;
    }
    let whole: i64 = [first]
        .into_iter()
        .chain(rest)
        .collect::<String>()
        .parse()
        .ok()?;
    let frac_cents: i64 = match frac.len() {
        0 => 0,
        1 => frac.parse::<i64>().ok()? * 10,
        _ => frac.parse().ok()?,
    };
    whole.checked_mul(100)?.checked_add(frac_cents).map(Some)
}

pub(super) fn fmt_cents(cents: i64) -> String {
    let (whole, frac) = (cents / 100, (cents % 100).abs());
    if frac == 0 {
        whole.to_string()
    } else {
        format!("{whole},{frac:02}")
    }
}

pub(super) fn fmt_percent(bp: u32) -> String {
    let (whole, frac) = (bp / 100, bp % 100);
    match (frac, frac % 10) {
        (0, _) => format!("{whole}"),
        (_, 0) => format!("{whole},{}", frac / 10),
        _ => format!("{whole},{frac:02}"),
    }
}

fn uyu(cents: i64) -> String {
    format_money(Money {
        cents,
        currency: Currency::Uyu,
    })
}

fn shipping_override(item: &ItemState) -> Option<Money> {
    if !item.shipping_user_set {
        return None;
    }
    shipping_money(&item.shipping_input)
}

fn shipping_money(input: &str) -> Option<Money> {
    parse_shipping(input).flatten().map(|cents| Money {
        cents,
        currency: Currency::Uyu,
    })
}

pub(super) fn input_style(
    invalid: bool,
) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |theme, status| {
        let mut style = theme::input(theme, status);
        if invalid {
            style.border.color = tokens(theme::is_dark(theme)).destructive;
        }
        style
    }
}

#[derive(Clone, Copy)]
pub(super) enum Tone {
    Muted,
    Ok,
    Bad,
}

pub(super) fn chip_style(tone: Tone) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let t = tokens(theme::is_dark(theme));
        let (bg, fg, border) = match tone {
            Tone::Muted => (t.muted, t.muted_foreground, t.border),
            Tone::Ok => (t.secondary, t.secondary_foreground, t.secondary),
            Tone::Bad => (Color::TRANSPARENT, t.destructive, t.destructive),
        };
        container::Style {
            text_color: Some(fg),
            background: Some(Background::Color(bg)),
            border: Border {
                color: border,
                width: 1.0,
                radius: theme::RADIUS.into(),
            },
            ..container::Style::default()
        }
    }
}

fn status_chip<'a>(lang: Lang, status: &ItemStatus) -> Option<Element<'a, Message>> {
    let (key, tone) = match status {
        ItemStatus::Idle => return None,
        ItemStatus::Scraping => (Key::StatusScraping, Tone::Muted),
        ItemStatus::Extracting => (Key::StatusExtracting, Tone::Muted),
        ItemStatus::Ready(_) => (Key::StatusReady, Tone::Ok),
        ItemStatus::Failed(_) => (Key::StatusFailed, Tone::Bad),
    };
    Some(
        container(text(t(lang, key)).size(12))
            .padding([4, 10])
            .style(chip_style(tone))
            .into(),
    )
}

fn cost_line(lang: Lang, product: &Product, cost: &CostBreakdown) -> String {
    let approx = |converted: bool| if converted { "\u{2248}" } else { "" };
    let mut parts = vec![format!(
        "{} {}{}",
        t(lang, Key::CostProduct),
        approx(product.price.currency == Currency::Usd),
        uyu(cost.product)
    )];
    parts.push(match cost.shipping {
        ShippingCost::Known(c) => format!("{} {}", t(lang, Key::CostShipping), uyu(c)),
        ShippingCost::Unknown => format!(
            "{} {}",
            t(lang, Key::CostShipping),
            t(lang, Key::ShippingToConfirm)
        ),
    });
    if let Some(tax) = cost.tax {
        let label = match tax.kind {
            TaxKind::Iva => Key::CostIva,
            TaxKind::RegimenSimplificado => Key::CostRegimen,
        };
        parts.push(format!(
            "{} {}% {}",
            t(lang, label),
            fmt_percent(tax.bp),
            uyu(tax.amount)
        ));
    }
    parts.push(format!(
        "{} {}{}",
        t(lang, Key::CostTotal),
        approx(cost.converted),
        uyu(cost.total)
    ));
    parts.join(" \u{00B7} ")
}

async fn resolve_fx(
    store: Option<Arc<crate::store::Store>>,
    manual: Option<f64>,
) -> Result<crate::domain::FxRate, String> {
    let cached = match &store {
        Some(s) => s.cached_fx().await.ok().flatten(),
        None => None,
    };
    if let RatePick::Use(fx) = pick_rate(cached.clone(), jiff::Zoned::now().date(), manual) {
        return Ok(fx);
    }
    match fetch_rate(&reqwest::Client::new(), DEFAULT_BASE_URL).await {
        Ok(fx) => {
            if let Some(s) = &store
                && let Err(e) = s.save_fx(&fx).await
            {
                eprintln!("maeve: cannot cache exchange rate: {e}");
            }
            Ok(fx)
        }
        // A stale automatic rate beats blocking the comparison offline.
        Err(e) => cached
            .filter(|c| !c.manual && validate_fx(c).is_ok())
            .ok_or(e),
    }
}

impl App {
    pub(super) fn running(&self) -> bool {
        matches!(self.run, RunState::Running { .. })
    }

    pub(super) fn load_input(&mut self, input: &CompareInput) {
        self.fx = Some(input.fx.clone());
        self.preset = input.preset.clone();
        self.purpose = input.purpose.clone().unwrap_or_default();
        self.items = input
            .items
            .iter()
            .map(|i| {
                let mut item = ItemState::new(i.url.clone());
                if let Some(p) = &i.product {
                    if i.shipping_override.is_some() {
                        item.shipping_input = self.default_shipping(p, i.shipping_override);
                        item.shipping_user_set = true;
                    }
                    item.status = ItemStatus::Ready(Box::new(p.clone()));
                }
                item
            })
            .collect();
    }

    fn default_shipping(&self, p: &Product, over: Option<Money>) -> String {
        let Some(fx) = &self.fx else {
            return String::new();
        };
        match landed_cost(p, over, fx, &self.settings.cost).shipping {
            ShippingCost::Known(c) => fmt_cents(c),
            ShippingCost::Unknown => String::new(),
        }
    }

    pub(super) fn breakdown(&self, i: usize) -> Option<CostBreakdown> {
        let item = self.items.get(i)?;
        let ItemStatus::Ready(p) = &item.status else {
            return None;
        };
        let fx = self.fx.as_ref()?;
        Some(landed_cost(
            p,
            shipping_override(item),
            fx,
            &self.settings.cost,
        ))
    }

    fn shipping_text(&self, i: usize) -> String {
        let Some(item) = self.items.get(i) else {
            return String::new();
        };
        match &item.status {
            _ if item.shipping_user_set => item.shipping_input.clone(),
            ItemStatus::Ready(p) => self.default_shipping(p, None),
            _ => String::new(),
        }
    }

    pub(super) fn load_presets(&self) -> Task<Message> {
        let Some(store) = self.store.clone() else {
            return Task::none();
        };
        Task::perform(
            async move {
                store.list_presets().await.unwrap_or_else(|e| {
                    eprintln!("maeve: cannot load presets: {e}");
                    Vec::new()
                })
            },
            Message::PresetsLoaded,
        )
    }

    /// Keeps the client (and its rate limiter) unless the key or rpm changed.
    pub(super) fn sync_llm(&mut self) {
        let Some(key) = self.api_key.clone() else {
            self.llm = None;
            return;
        };
        let rpm = self.settings.rpm;
        if self
            .llm
            .as_ref()
            .is_some_and(|h| h.key == key && h.rpm == rpm)
        {
            return;
        }
        self.llm = Some(LlmHandle {
            client: Arc::new(GeminiClient::new(key.clone(), rpm)),
            key,
            rpm,
        });
    }

    pub(super) fn set_api_key(&mut self, key: Option<String>) {
        self.api_key = key.filter(|k| !k.trim().is_empty());
        self.sync_llm();
    }

    fn preset_to_save(&self) -> Option<Preset> {
        let Preset::Custom { criteria, .. } = &self.preset else {
            return None;
        };
        let name: String = self
            .preset_name
            .trim()
            .chars()
            .take(MAX_PRESET_NAME)
            .collect();
        let criteria = criteria.trim();
        (is_ad_hoc(&self.preset)
            && !name.is_empty()
            && !name.eq_ignore_ascii_case(AD_HOC_NAME)
            && !criteria.is_empty())
        .then(|| Preset::Custom {
            name,
            criteria: criteria.to_owned(),
        })
    }

    pub(super) fn save_preset(&mut self, preset: Preset) -> Task<Message> {
        let Preset::Custom { name, criteria } = &preset else {
            return Task::none();
        };
        if name.trim().is_empty()
            || name.eq_ignore_ascii_case(AD_HOC_NAME)
            || criteria.trim().is_empty()
        {
            return Task::none();
        }
        let Some(store) = self.store.clone() else {
            return Task::none();
        };
        Task::perform(
            async move {
                store
                    .save_preset(&preset)
                    .await
                    .map(|()| preset)
                    .map_err(|e| e.to_string())
            },
            Message::PresetSaved,
        )
    }

    pub(super) fn preset_saved(&mut self, result: Result<Preset, String>) {
        match result {
            Ok(saved) => {
                let Preset::Custom { name, .. } = &saved else {
                    return;
                };
                self.custom_presets
                    .retain(|p| !matches!(p, Preset::Custom { name: n, .. } if n == name));
                self.custom_presets.push(saved.clone());
                self.custom_presets.sort_by(|a, b| match (a, b) {
                    (Preset::Custom { name: a, .. }, Preset::Custom { name: b, .. }) => a.cmp(b),
                    _ => std::cmp::Ordering::Equal,
                });
                self.preset = saved;
                self.preset_name.clear();
            }
            Err(e) => eprintln!("maeve: cannot save preset: {e}"),
        }
    }

    pub(super) fn url_changed(&mut self, i: usize, url: String) {
        if self.running() {
            return;
        }
        if let Some(item) = self.items.get_mut(i) {
            item.url = url;
            item.status = ItemStatus::Idle;
            item.shipping_input.clear();
            item.shipping_user_set = false;
            item.shipping_invalid = false;
            item.paste_error = None;
        }
    }

    pub(super) fn add_item(&mut self) {
        if !self.running() {
            self.items.push(ItemState::new(""));
        }
    }

    pub(super) fn remove_item(&mut self, i: usize) {
        if !self.running() && !self.extracting() && self.items.len() > 2 && i < self.items.len() {
            self.items.remove(i);
            self.paste_row = match self.paste_row {
                Some(r) if r == i => None,
                Some(r) if r > i => Some(r - 1),
                other => other,
            };
        }
    }

    pub(super) fn shipping_changed(&mut self, i: usize, value: &str) {
        if self.running() {
            return;
        }
        let valid = parse_shipping(value).is_some();
        let Some(item) = self.items.get_mut(i) else {
            return;
        };
        item.shipping_invalid = !valid;
        if valid {
            item.shipping_input = value.to_owned();
            item.shipping_user_set = true;
            self.stale |= self.result.is_some();
        }
    }

    fn can_compare(&self) -> bool {
        let criteria_ok = match &self.preset {
            Preset::Custom { criteria, .. } if is_ad_hoc(&self.preset) => {
                !criteria.trim().is_empty()
            }
            _ => true,
        };
        !self.running()
            && !self.extracting()
            && criteria_ok
            && self
                .items
                .iter()
                .filter(|i| !i.url.trim().is_empty())
                .count()
                >= 2
    }

    pub(super) fn compare(&mut self) -> Task<Message> {
        if !self.can_compare() {
            return Task::none();
        }
        if self.llm.is_none() {
            self.run = RunState::Error(RunError::NoApiKey);
            return Task::none();
        }
        self.run_items = (0..self.items.len())
            .filter(|&i| !self.items[i].url.trim().is_empty())
            .collect();
        for &i in &self.run_items {
            let item = &mut self.items[i];
            item.paste_error = None;
            if !matches!(item.status, ItemStatus::Ready(_)) {
                item.status = ItemStatus::Scraping;
            }
        }
        self.paste_row = None;
        self.run = RunState::Running {
            advocates_done: 0,
            waiting: None,
            judging: false,
        };
        let store = self.store.clone();
        Task::perform(
            resolve_fx(store, self.settings.fx_manual),
            Message::FxResolved,
        )
    }

    fn abort_run(&mut self, error: RunError) {
        for item in &mut self.items {
            if matches!(item.status, ItemStatus::Scraping) {
                item.status = ItemStatus::Idle;
            }
        }
        self.run = RunState::Error(error);
    }

    pub(super) fn fx_resolved(
        &mut self,
        fx: Result<crate::domain::FxRate, String>,
    ) -> Task<Message> {
        if !self.running() {
            return Task::none();
        }
        let fx = match fx {
            Ok(fx) => fx,
            Err(e) => {
                self.abort_run(RunError::Other(e));
                return Task::none();
            }
        };
        let Some(llm) = self.llm.as_ref().map(|h| Arc::clone(&h.client)) else {
            self.abort_run(RunError::NoApiKey);
            return Task::none();
        };
        self.fx = Some(fx.clone());
        let input = self.build_input(fx);
        self.input = Some(input.clone());
        Task::run(
            run_comparison(Arc::clone(&self.scraper), llm, input),
            |event| Message::Pipeline(Box::new(event)),
        )
    }

    fn build_input(&self, fx: crate::domain::FxRate) -> CompareInput {
        let items = self
            .run_items
            .iter()
            .map(|&i| {
                let item = &self.items[i];
                ItemInput {
                    url: item.url.trim().to_owned(),
                    product: match &item.status {
                        ItemStatus::Ready(p) => Some((**p).clone()),
                        _ => None,
                    },
                    shipping_override: shipping_override(item),
                }
            })
            .collect();
        let purpose = self.purpose.trim();
        CompareInput {
            items,
            preset: self.preset.clone(),
            purpose: (!purpose.is_empty()).then(|| purpose.to_owned()),
            lang: self.settings.lang,
            cost: self.settings.cost.clone(),
            fx,
            advocate_model: self.settings.advocate_model.clone(),
            judge_model: self.settings.judge_model.clone(),
        }
    }

    pub(super) fn on_pipeline(&mut self, event: PipelineEvent) -> Task<Message> {
        if !self.running() {
            return Task::none();
        }
        match event {
            PipelineEvent::Scraped { item, result } => {
                let Some(&i) = self.run_items.get(item) else {
                    return Task::none();
                };
                self.items[i].status = match result {
                    Ok(p) => ItemStatus::Ready(Box::new(p)),
                    Err(e) => ItemStatus::Failed(e),
                };
            }
            PipelineEvent::AdvocateStarted { .. } => {}
            PipelineEvent::AdvocateDone { .. } => {
                if let RunState::Running {
                    advocates_done,
                    waiting,
                    ..
                } = &mut self.run
                {
                    *advocates_done += 1;
                    *waiting = None;
                }
            }
            PipelineEvent::Waiting(d) => {
                if let RunState::Running { waiting, .. } = &mut self.run {
                    *waiting = Some(d);
                }
            }
            PipelineEvent::JudgeStarted => {
                if let RunState::Running {
                    waiting, judging, ..
                } = &mut self.run
                {
                    *waiting = None;
                    *judging = true;
                }
            }
            PipelineEvent::Finished(Ok(result)) => return self.finish_run(result),
            PipelineEvent::Finished(Err(e)) => {
                self.run = RunState::Error(match e {
                    PipelineError::NotEnoughProducts => RunError::NotEnoughProducts,
                    PipelineError::Llm(LlmError::Auth) => RunError::Auth,
                    other => RunError::Other(other.to_string()),
                });
            }
        }
        Task::none()
    }

    pub(super) fn retry(&mut self, i: usize) -> Task<Message> {
        if !self.can_compare() {
            return Task::none();
        }
        if let Some(item) = self.items.get_mut(i)
            && matches!(item.status, ItemStatus::Failed(_))
        {
            item.status = ItemStatus::Idle;
        }
        self.compare()
    }

    fn progress_text(&self) -> Option<String> {
        let lang = self.settings.lang;
        let RunState::Running {
            advocates_done,
            waiting,
            judging,
        } = &self.run
        else {
            return None;
        };
        let scraping = self
            .run_items
            .iter()
            .any(|&i| matches!(self.items[i].status, ItemStatus::Scraping));
        let mut line = if scraping {
            t(lang, Key::ProgressScraping).to_owned()
        } else if *judging {
            t(lang, Key::ProgressJudging).to_owned()
        } else {
            let total = self
                .run_items
                .iter()
                .filter(|&&i| matches!(self.items[i].status, ItemStatus::Ready(_)))
                .count();
            format!(
                "{} {advocates_done}/{total}",
                t(lang, Key::ProgressAdvocates)
            )
        };
        if let Some(wait) = waiting {
            line.push_str(&format!(
                " \u{00B7} {} ({} s)",
                t(lang, Key::ProgressWaiting),
                wait_secs(*wait)
            ));
        }
        Some(line)
    }

    fn preset_chips(&self) -> Element<'_, Message> {
        let lang = self.settings.lang;
        let running = self.running();
        let chip = |label: String, selected: bool, preset: Preset| {
            button(text(label).size(13))
                .padding([6, 14])
                .style(theme::chip(selected))
                .on_press_maybe((!running).then_some(Message::PresetPicked(preset)))
        };
        let builtin = [
            (Key::PresetBestOverall, Preset::BestOverall),
            (Key::PresetBestValue, Preset::QualityPrice),
            (Key::PresetCheapest, Preset::CheapestDecent),
        ];
        let mut chips: Vec<Element<'_, Message>> = builtin
            .into_iter()
            .map(|(key, preset)| {
                chip(t(lang, key).to_owned(), self.preset == preset, preset).into()
            })
            .collect();
        for saved in &self.custom_presets {
            if let Preset::Custom { name, .. } = saved {
                chips.push(chip(name.clone(), self.preset == *saved, saved.clone()).into());
            }
        }
        let criteria = match &self.preset {
            Preset::Custom { criteria, .. } if is_ad_hoc(&self.preset) => criteria.clone(),
            _ => String::new(),
        };
        chips.push(
            chip(
                t(lang, Key::PresetCustom).to_owned(),
                is_ad_hoc(&self.preset),
                ad_hoc_preset(criteria),
            )
            .into(),
        );
        row(chips).spacing(8).wrap().into()
    }

    fn item_row(&self, i: usize) -> Element<'_, Message> {
        let lang = self.settings.lang;
        let t_ = tokens(theme::is_dark(&self.theme()));
        let running = self.running();
        let extracting = self.extracting();
        let item = &self.items[i];

        let url = text_input(t(lang, Key::UrlPlaceholder), &item.url)
            .on_input_maybe((!running).then_some(move |s: String| Message::UrlChanged(i, s)))
            .padding([8, 12])
            .size(14)
            .style(input_style(false))
            .width(Length::Fill);

        let breakdown = self.breakdown(i);
        let shipping_text = self.shipping_text(i);
        let unknown_shipping = shipping_text.trim().is_empty()
            && breakdown
                .as_ref()
                .is_some_and(|b| b.shipping == ShippingCost::Unknown);
        let shipping = text_input(t(lang, Key::ShippingPlaceholder), &shipping_text)
            .on_input_maybe((!running).then_some(move |s: String| Message::ShippingChanged(i, s)))
            .padding([8, 12])
            .size(14)
            .style(input_style(item.shipping_invalid || unknown_shipping))
            .width(120);

        let mut top = row![url].spacing(8).align_y(Alignment::Center);
        if let Some(chip) = status_chip(lang, &item.status) {
            top = top.push(chip);
        }
        top = top.push(shipping);
        if self.items.len() > 2 {
            top = top.push(
                button(text(t(lang, Key::RemoveUrl)).size(13))
                    .padding([6, 10])
                    .style(theme::button_ghost)
                    .on_press_maybe((!running && !extracting).then_some(Message::RemoveItem(i))),
            );
        }

        let mut col = column![top].spacing(4);
        match &item.status {
            ItemStatus::Failed(e) => {
                let can_paste = e.kind != ScrapeErrorKind::Unsupported;
                let mut actions = row![
                    text(e.message.clone()).size(13).color(t_.destructive),
                    Space::new().width(Length::Fill),
                    button(text(t(lang, Key::Retry)).size(13))
                        .padding([4, 10])
                        .style(theme::button_secondary)
                        .on_press_maybe((!running).then_some(Message::Retry(i))),
                ]
                .spacing(8)
                .align_y(Alignment::Center);
                if can_paste {
                    actions = actions.push(
                        button(text(t(lang, Key::PasteInstead)).size(13))
                            .padding([4, 10])
                            .style(theme::button_secondary)
                            .on_press_maybe((!running).then_some(Message::PasteMode(i))),
                    );
                }
                col = col.push(actions);
                if can_paste && self.paste_row == Some(i) {
                    col = col.push(
                        text(t(lang, Key::PasteHint))
                            .size(13)
                            .style(theme::text_muted),
                    );
                    col = col.push(
                        row![
                            button(text(t(lang, Key::PasteText)).size(13))
                                .padding([4, 10])
                                .style(theme::button_secondary)
                                .on_press_maybe((!running).then_some(Message::PasteText(i))),
                            button(text(t(lang, Key::PasteImage)).size(13))
                                .padding([4, 10])
                                .style(theme::button_secondary)
                                .on_press_maybe((!running).then_some(Message::PasteImage(i))),
                        ]
                        .spacing(8),
                    );
                }
                if let Some(error) = &item.paste_error {
                    col = col.push(text(error.clone()).size(13).color(t_.destructive));
                }
            }
            ItemStatus::Ready(p) => {
                if let Some(b) = &breakdown {
                    col = col.push(
                        text(cost_line(lang, p, b))
                            .size(13)
                            .style(theme::text_muted),
                    );
                }
            }
            _ => {}
        }
        col.into()
    }

    pub(super) fn input_card(&self) -> Element<'_, Message> {
        let lang = self.settings.lang;
        let t_ = tokens(theme::is_dark(&self.theme()));
        let running = self.running();

        let mut items = column![].spacing(12);
        for i in 0..self.items.len() {
            items = items.push(self.item_row(i));
        }
        let add = button(text(t(lang, Key::AddUrl)).size(13))
            .padding([6, 10])
            .style(theme::button_ghost)
            .on_press_maybe((!running).then_some(Message::AddItem));

        let mut card = column![items, add, self.preset_chips()].spacing(16);
        if let Preset::Custom { criteria, .. } = &self.preset
            && is_ad_hoc(&self.preset)
        {
            let save = self.preset_to_save();
            card = card.push(
                row![
                    text_input(t(lang, Key::CustomCriteriaPlaceholder), criteria)
                        .on_input_maybe((!running).then_some(Message::CriteriaChanged))
                        .padding([8, 12])
                        .size(14)
                        .style(input_style(false))
                        .width(Length::Fill),
                    text_input(t(lang, Key::PresetNamePlaceholder), &self.preset_name)
                        .on_input_maybe((!running).then_some(Message::PresetNameChanged))
                        .padding([8, 12])
                        .size(14)
                        .style(input_style(false))
                        .width(180),
                    button(text(t(lang, Key::PresetSave)).size(13))
                        .padding([8, 14])
                        .style(theme::button_secondary)
                        .on_press_maybe(
                            save.filter(|_| !running && self.store.is_some())
                                .map(Message::SavePreset)
                        ),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        card = card.push(
            text_input(t(lang, Key::PurposePlaceholder), &self.purpose)
                .on_input_maybe((!running).then_some(Message::PurposeChanged))
                .padding([8, 12])
                .size(14)
                .style(input_style(false)),
        );

        let mut action = row![
            button(text(t(lang, Key::Compare)).size(14))
                .padding([9, 24])
                .style(theme::button_primary)
                .on_press_maybe(self.can_compare().then_some(Message::Compare))
        ]
        .spacing(16)
        .align_y(Alignment::Center);
        match &self.run {
            RunState::Error(e) => {
                let msg = match e {
                    RunError::NoApiKey => t(lang, Key::NoApiKey).to_owned(),
                    RunError::NotEnoughProducts => t(lang, Key::NotEnoughProducts).to_owned(),
                    RunError::Auth => t(lang, Key::ApiKeyRejected).to_owned(),
                    RunError::Other(m) => m.clone(),
                };
                action = action.push(text(msg).size(13).color(t_.destructive));
            }
            _ => {
                if let Some(line) = self.progress_text() {
                    action = action.push(text(line).size(13).style(theme::text_muted));
                }
            }
        }
        card = card.push(action);

        container(card)
            .padding(24)
            .width(Length::Fill)
            .style(theme::card)
            .into()
    }
}

fn wait_secs(d: Duration) -> u64 {
    d.as_secs_f64().ceil() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::demo_comparison;
    use crate::domain::FxRate;
    use crate::scrape::{ScrapeError, ScrapeErrorKind};
    use crate::settings::Settings;
    use crate::ui::snap::DevEnv;

    fn blank_app() -> App {
        App::new(Settings::default(), None, DevEnv::default())
    }

    fn demo_app() -> App {
        let dev = DevEnv {
            demo: true,
            ..DevEnv::default()
        };
        App::new(Settings::default(), None, dev)
    }

    fn two_urls(app: &mut App) {
        app.items[0].url = "https://a.example/1".into();
        app.items[1].url = "https://b.example/2".into();
    }

    fn keyed(mut app: App) -> App {
        app.set_api_key(Some("test-key".into()));
        app
    }

    fn fx() -> FxRate {
        FxRate {
            uyu_per_usd: 40.0,
            date: jiff::civil::date(2026, 11, 12),
            manual: false,
        }
    }

    fn pipeline(app: &mut App, event: PipelineEvent) {
        let _ = app.update(Message::Pipeline(Box::new(event)));
    }

    fn scrape_err() -> ScrapeError {
        ScrapeError::new(ScrapeErrorKind::Blocked, "blocked")
    }

    #[test]
    fn compare_with_one_url_is_a_noop() {
        let mut app = keyed(blank_app());
        app.items[0].url = "https://a.example/1".into();
        let task = app.update(Message::Compare);
        assert_eq!(task.units(), 0);
        assert_eq!(app.run, RunState::Idle);
    }

    #[test]
    fn compare_ignores_blank_urls() {
        let mut app = keyed(blank_app());
        app.items[0].url = "https://a.example/1".into();
        app.items[1].url = "   ".into();
        let task = app.update(Message::Compare);
        assert_eq!(task.units(), 0);
        assert_eq!(app.run, RunState::Idle);
    }

    #[test]
    fn compare_without_key_errors_and_spawns_nothing() {
        let mut app = blank_app();
        two_urls(&mut app);
        let task = app.update(Message::Compare);
        assert_eq!(task.units(), 0);
        assert_eq!(app.run, RunState::Error(RunError::NoApiKey));
        assert!(
            app.items
                .iter()
                .all(|i| matches!(i.status, ItemStatus::Idle))
        );
    }

    #[test]
    fn compare_marks_running_and_blocks_a_second_run() {
        let mut app = keyed(blank_app());
        two_urls(&mut app);
        let task = app.update(Message::Compare);
        assert_eq!(task.units(), 1);
        assert!(matches!(app.run, RunState::Running { .. }));
        assert!(
            app.items
                .iter()
                .all(|i| matches!(i.status, ItemStatus::Scraping))
        );
        assert_eq!(app.update(Message::Compare).units(), 0);
    }

    #[test]
    fn resolved_rate_starts_the_pipeline() {
        let mut app = keyed(blank_app());
        two_urls(&mut app);
        let _ = app.update(Message::Compare);
        let task = app.update(Message::FxResolved(Ok(fx())));
        assert_eq!(task.units(), 1);
        let input = app.input.as_ref().unwrap();
        assert_eq!(input.items.len(), 2);
        assert!(input.items.iter().all(|i| i.product.is_none()));
        assert_eq!(app.fx, Some(fx()));
    }

    #[test]
    fn rate_failure_reports_and_resets_scraping() {
        let mut app = keyed(blank_app());
        two_urls(&mut app);
        let _ = app.update(Message::Compare);
        let task = app.update(Message::FxResolved(Err("offline".into())));
        assert_eq!(task.units(), 0);
        assert_eq!(app.run, RunState::Error(RunError::Other("offline".into())));
        assert!(
            app.items
                .iter()
                .all(|i| matches!(i.status, ItemStatus::Idle))
        );
    }

    #[test]
    fn scrape_failure_marks_item_failed() {
        let mut app = keyed(blank_app());
        two_urls(&mut app);
        let _ = app.update(Message::Compare);
        pipeline(
            &mut app,
            PipelineEvent::Scraped {
                item: 1,
                result: Err(scrape_err()),
            },
        );
        assert!(matches!(app.items[1].status, ItemStatus::Failed(_)));
        assert!(matches!(app.items[0].status, ItemStatus::Scraping));
    }

    #[test]
    fn pipeline_indices_follow_non_empty_rows() {
        let mut app = keyed(blank_app());
        app.items = vec![
            ItemState::new("https://a.example/1"),
            ItemState::new(""),
            ItemState::new("https://b.example/2"),
        ];
        let _ = app.update(Message::Compare);
        pipeline(
            &mut app,
            PipelineEvent::Scraped {
                item: 1,
                result: Err(scrape_err()),
            },
        );
        assert!(matches!(app.items[2].status, ItemStatus::Failed(_)));
        assert!(matches!(app.items[1].status, ItemStatus::Idle));
    }

    #[test]
    fn scraped_product_prefills_shipping_and_cost() {
        let mut app = keyed(blank_app());
        two_urls(&mut app);
        let _ = app.update(Message::Compare);
        let _ = app.update(Message::FxResolved(Ok(fx())));
        let product = demo_comparison().1.products[1].clone();
        pipeline(
            &mut app,
            PipelineEvent::Scraped {
                item: 1,
                result: Ok(product),
            },
        );
        assert!(matches!(app.items[1].status, ItemStatus::Ready(_)));
        assert_eq!(app.shipping_text(1), "0");
        assert!(!app.items[1].shipping_user_set);
        assert!(app.breakdown(1).is_some());
    }

    #[test]
    fn progress_tracks_advocates_waiting_and_judge() {
        let mut app = keyed(demo_app());
        let _ = app.update(Message::Compare);
        let _ = app.update(Message::FxResolved(Ok(fx())));
        assert_eq!(app.progress_text().unwrap(), "Defensores 0/2");
        pipeline(
            &mut app,
            PipelineEvent::Waiting(Duration::from_millis(11_200)),
        );
        assert_eq!(
            app.progress_text().unwrap(),
            "Defensores 0/2 \u{00B7} esperando límite de Gemini (12 s)"
        );
        pipeline(&mut app, PipelineEvent::AdvocateDone { item: 0 });
        assert_eq!(app.progress_text().unwrap(), "Defensores 1/2");
        pipeline(&mut app, PipelineEvent::JudgeStarted);
        assert_eq!(app.progress_text().unwrap(), "El juez está decidiendo…");
    }

    #[test]
    fn finishing_stores_result_and_clears_stale() {
        let mut app = keyed(demo_app());
        app.stale = true;
        let _ = app.update(Message::Compare);
        let _ = app.update(Message::FxResolved(Ok(fx())));
        let result = demo_comparison().1;
        pipeline(&mut app, PipelineEvent::Finished(Ok(result.clone())));
        assert_eq!(app.run, RunState::Idle);
        assert!(!app.stale);
        assert_eq!(app.result, Some(result));
    }

    #[test]
    fn pipeline_errors_map_to_run_errors() {
        let mut app = keyed(demo_app());
        let _ = app.update(Message::Compare);
        pipeline(
            &mut app,
            PipelineEvent::Finished(Err(PipelineError::NotEnoughProducts)),
        );
        assert_eq!(app.run, RunState::Error(RunError::NotEnoughProducts));

        let _ = app.update(Message::Compare);
        pipeline(
            &mut app,
            PipelineEvent::Finished(Err(PipelineError::Llm(LlmError::Auth))),
        );
        assert_eq!(app.run, RunState::Error(RunError::Auth));
    }

    #[test]
    fn rerun_reuses_ready_products() {
        let mut app = keyed(demo_app());
        let _ = app.update(Message::Compare);
        let _ = app.update(Message::FxResolved(Ok(fx())));
        let input = app.input.as_ref().unwrap();
        assert!(input.items.iter().all(|i| i.product.is_some()));
    }

    #[test]
    fn retry_rescrapes_only_the_failed_item() {
        let mut app = keyed(demo_app());
        app.items[1].status = ItemStatus::Failed(scrape_err());
        let task = app.update(Message::Retry(1));
        assert_eq!(task.units(), 1);
        assert!(matches!(app.items[0].status, ItemStatus::Ready(_)));
        assert!(matches!(app.items[1].status, ItemStatus::Scraping));
    }

    #[test]
    fn shipping_edit_recomputes_and_marks_stale_without_running() {
        let mut app = keyed(demo_app());
        let before = app.breakdown(1).unwrap();
        assert_eq!(before.shipping, ShippingCost::Known(0));

        let task = app.update(Message::ShippingChanged(1, "150".into()));
        assert_eq!(task.units(), 0);
        assert_eq!(app.run, RunState::Idle);
        assert!(app.stale);
        assert!(!app.items[1].shipping_invalid);
        let after = app.breakdown(1).unwrap();
        assert_eq!(after.shipping, ShippingCost::Known(15_000));
        assert!(after.total > before.total);
    }

    #[test]
    fn shipping_edit_without_result_is_not_stale() {
        let mut app = keyed(demo_app());
        app.result = None;
        let _ = app.update(Message::ShippingChanged(1, "150".into()));
        assert!(!app.stale);
    }

    #[test]
    fn invalid_shipping_keeps_previous_value_and_flags() {
        let mut app = demo_app();
        let _ = app.update(Message::ShippingChanged(1, "150".into()));
        app.stale = false;
        let _ = app.update(Message::ShippingChanged(1, "abc".into()));
        assert_eq!(app.items[1].shipping_input, "150");
        assert!(app.items[1].shipping_invalid);
        assert!(!app.stale);
        let _ = app.update(Message::ShippingChanged(1, "1.500".into()));
        assert!(!app.items[1].shipping_invalid);
        assert_eq!(app.items[1].shipping_input, "1.500");
    }

    #[test]
    fn default_shipping_is_not_sent_as_an_override() {
        let mut app = keyed(demo_app());
        let _ = app.update(Message::Compare);
        let _ = app.update(Message::FxResolved(Ok(fx())));
        let input = app.input.as_ref().unwrap();
        assert!(input.items.iter().all(|i| i.shipping_override.is_none()));
    }

    #[test]
    fn default_shipping_follows_the_rate() {
        let mut app = demo_app();
        let mut product = demo_comparison().1.products[1].clone();
        product.shipping = Some(Money {
            cents: 500,
            currency: Currency::Usd,
        });
        app.items[1].status = ItemStatus::Ready(Box::new(product));
        app.fx = Some(fx());
        assert_eq!(app.shipping_text(1), "200");
        app.fx = Some(FxRate {
            uyu_per_usd: 42.0,
            ..fx()
        });
        assert_eq!(app.shipping_text(1), "210");
    }

    #[test]
    fn shipping_override_goes_to_the_pipeline() {
        let mut app = keyed(demo_app());
        let _ = app.update(Message::ShippingChanged(1, "150,50".into()));
        let _ = app.update(Message::Compare);
        let _ = app.update(Message::FxResolved(Ok(fx())));
        let over = app.input.as_ref().unwrap().items[1].shipping_override;
        assert_eq!(
            over,
            Some(Money {
                cents: 15_050,
                currency: Currency::Uyu
            })
        );
    }

    #[test]
    fn editing_a_url_drops_its_scraped_product() {
        let mut app = demo_app();
        let _ = app.update(Message::UrlChanged(0, "https://other.example/x".into()));
        assert!(matches!(app.items[0].status, ItemStatus::Idle));
        assert!(!app.items[0].shipping_user_set);
        assert!(app.items[0].shipping_input.is_empty());
        assert!(matches!(app.items[1].status, ItemStatus::Ready(_)));
    }

    #[test]
    fn rows_are_locked_while_running() {
        let mut app = keyed(demo_app());
        let _ = app.update(Message::Compare);
        let _ = app.update(Message::UrlChanged(0, "x".into()));
        let _ = app.update(Message::AddItem);
        let _ = app.update(Message::ShippingChanged(1, "99".into()));
        assert_eq!(app.items.len(), 2);
        assert!(app.items[0].url.starts_with("https://articulo"));
        assert_eq!(app.shipping_text(1), "0");
        assert!(!app.items[1].shipping_user_set);
    }

    #[test]
    fn at_least_two_rows_remain() {
        let mut app = blank_app();
        let _ = app.update(Message::RemoveItem(0));
        assert_eq!(app.items.len(), 2);
        let _ = app.update(Message::AddItem);
        assert_eq!(app.items.len(), 3);
        let _ = app.update(Message::RemoveItem(2));
        assert_eq!(app.items.len(), 2);
    }

    #[test]
    fn free_text_preset_is_named_for_history() {
        let mut app = blank_app();
        let _ = app.update(Message::CriteriaChanged("silent".into()));
        assert_eq!(
            app.preset,
            Preset::Custom {
                name: "custom".into(),
                criteria: "silent".into()
            }
        );
    }

    #[test]
    fn client_is_rebuilt_only_when_key_or_rpm_change() {
        let mut app = blank_app();
        assert!(app.llm.is_none());
        app.set_api_key(Some("k1".into()));
        let first = Arc::clone(&app.llm.as_ref().unwrap().client);
        app.sync_llm();
        assert!(Arc::ptr_eq(&first, &app.llm.as_ref().unwrap().client));

        app.settings.rpm = 5;
        app.sync_llm();
        let second = Arc::clone(&app.llm.as_ref().unwrap().client);
        assert!(!Arc::ptr_eq(&first, &second));

        app.set_api_key(Some("k2".into()));
        assert!(!Arc::ptr_eq(&second, &app.llm.as_ref().unwrap().client));

        app.set_api_key(Some("  ".into()));
        assert!(app.llm.is_none());
    }

    #[test]
    fn parses_uruguayan_amounts() {
        assert_eq!(parse_shipping(""), Some(None));
        assert_eq!(parse_shipping("  "), Some(None));
        assert_eq!(parse_shipping("150"), Some(Some(15_000)));
        assert_eq!(parse_shipping("1.500"), Some(Some(150_000)));
        assert_eq!(parse_shipping("1.234.567"), Some(Some(123_456_700)));
        assert_eq!(parse_shipping("150,5"), Some(Some(15_050)));
        assert_eq!(parse_shipping("150,50"), Some(Some(15_050)));
        assert_eq!(parse_shipping("150,"), Some(Some(15_000)));
        assert_eq!(parse_shipping("0"), Some(Some(0)));
    }

    #[test]
    fn rejects_malformed_amounts() {
        for bad in [
            "abc",
            "1.5",
            "1.50",
            "12.3456",
            ".5",
            ",5",
            "-5",
            "1,234",
            "1,2,3",
            "1..000",
            "1.000.00",
            "99999999999999999999",
        ] {
            assert_eq!(parse_shipping(bad), None, "{bad}");
        }
    }

    #[test]
    fn formats_cents_and_percentages() {
        assert_eq!(fmt_cents(15_000), "150");
        assert_eq!(fmt_cents(15_050), "150,50");
        assert_eq!(fmt_cents(0), "0");
        assert_eq!(fmt_percent(2200), "22");
        assert_eq!(fmt_percent(2250), "22,5");
        assert_eq!(fmt_percent(2205), "22,05");
    }

    fn custom(name: &str, criteria: &str) -> Preset {
        Preset::Custom {
            name: name.into(),
            criteria: criteria.into(),
        }
    }

    #[tokio::test]
    async fn custom_preset_is_saved_through_the_store_and_selected() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            crate::store::Store::open(&dir.path().join("t.db"))
                .await
                .unwrap(),
        );
        let mut app = App::new(
            Settings::default(),
            Some(Arc::clone(&store)),
            DevEnv::default(),
        );
        let _ = app.update(Message::CriteriaChanged("silent".into()));
        let _ = app.update(Message::PresetNameChanged("  Quiet  ".into()));
        let preset = custom("Quiet", "silent");
        assert_eq!(app.preset_to_save(), Some(preset.clone()));

        assert_eq!(app.update(Message::SavePreset(preset.clone())).units(), 1);
        store.save_preset(&preset).await.unwrap();
        let _ = app.update(Message::PresetSaved(Ok(preset.clone())));
        assert_eq!(app.preset, preset);
        assert_eq!(app.custom_presets, vec![preset.clone()]);
        assert!(app.preset_name.is_empty());
        assert_eq!(store.list_presets().await.unwrap(), vec![preset]);
    }

    #[test]
    fn preset_needs_a_real_name_and_criteria() {
        let mut app = blank_app();
        let _ = app.update(Message::CriteriaChanged("silent".into()));
        assert_eq!(app.preset_to_save(), None);
        for name in ["custom", "CUSTOM", "  "] {
            let _ = app.update(Message::PresetNameChanged(name.into()));
            assert_eq!(app.preset_to_save(), None, "{name:?}");
            let task = app.update(Message::SavePreset(custom(name, "silent")));
            assert_eq!(task.units(), 0);
        }
        let _ = app.update(Message::PresetNameChanged("Quiet".into()));
        assert!(app.preset_to_save().is_some());
        let _ = app.update(Message::CriteriaChanged("  ".into()));
        assert_eq!(app.preset_to_save(), None);
        let _ = app.update(Message::PresetPicked(Preset::BestOverall));
        assert_eq!(app.preset_to_save(), None);
    }

    #[test]
    fn resaving_a_name_replaces_the_chip() {
        let mut app = blank_app();
        app.custom_presets = vec![custom("Quiet", "old"), custom("Zed", "z")];
        let _ = app.update(Message::PresetSaved(Ok(custom("Quiet", "new"))));
        assert_eq!(
            app.custom_presets,
            vec![custom("Quiet", "new"), custom("Zed", "z")]
        );
    }
}
