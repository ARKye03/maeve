//! Results card: verdict, stats table, bar charts and the advocates' debate.

use std::sync::Arc;

use iced::font::Weight;
use iced::widget::tooltip::Position;
use iced::widget::{Space, button, canvas, column, container, row, rule, text, tooltip};
use iced::{
    Alignment, Background, Border, Color, Element, Font, Length, Point, Rectangle, Renderer, Size,
    Task, Theme, mouse,
};

use super::input::{Tone, chip_style};
use super::{App, ItemStatus, Message};
use crate::domain::{RowKind, ShippingCost, Site, StatRow, stats::units_agree};
use crate::i18n::{Key, Lang, t};
use crate::llm::schema::{AlignedRow, Better};
use crate::pipeline::ComparisonResult;
use crate::store::{ComparisonRecord, Store};
use crate::theme::{self, tokens};

const TITLE_CHARS: usize = 28;
const BAR_HEIGHT: f32 = 12.0;
const BAR_LABEL_WIDTH: f32 = 190.0;
const BAR_VALUE_WIDTH: f32 = 110.0;

/// The default sans-serif face has no bold cut here and falls back to a
/// monospace one, so name a family that ships one on macOS (the only target).
pub(super) fn bold() -> Font {
    Font {
        weight: Weight::Bold,
        ..Font::with_name("Helvetica Neue")
    }
}

pub(super) fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}\u{2026}", cut.trim_end())
}

fn site_label(site: Site) -> &'static str {
    match site {
        Site::MercadoLibre => "Mercado Libre",
        Site::Temu => "TEMU",
    }
}

fn row_label(lang: Lang, row: &StatRow) -> String {
    match row.kind {
        RowKind::Total => t(lang, Key::CostTotal).to_owned(),
        RowKind::Rating => t(lang, Key::RowRating).to_owned(),
        RowKind::Reviews => t(lang, Key::RowReviews).to_owned(),
        RowKind::Spec => row.label.clone(),
    }
}

fn better(row: &StatRow, judge_rows: &[AlignedRow]) -> Better {
    match row.kind {
        RowKind::Total => Better::Lower,
        RowKind::Rating | RowKind::Reviews => Better::Higher,
        RowKind::Spec => judge_rows
            .iter()
            .find(|r| r.label == row.label)
            .map_or(Better::None, |r| r.better),
    }
}

/// `None` when the cell is missing; converted totals carry the "≈" marker.
fn cell_text(result: &ComparisonResult, row: &StatRow, i: usize) -> Option<String> {
    let raw = row.cells.get(i)?.raw.as_deref()?;
    let converted = row.kind == RowKind::Total && result.costs.get(i).is_some_and(|c| c.converted);
    Some(if converted {
        format!("\u{2248} {raw}")
    } else {
        raw.to_owned()
    })
}

/// Values worth charting: every product has a number and the units line up.
fn bar_values(row: &StatRow, judge_rows: &[AlignedRow]) -> Option<Vec<f64>> {
    if row.cells.len() < 2 {
        return None;
    }
    let values = row
        .cells
        .iter()
        .map(|c| c.numeric.filter(|n| n.is_finite() && *n >= 0.0))
        .collect::<Option<Vec<f64>>>()?;
    if row.kind == RowKind::Spec
        && let Some(aligned) = judge_rows.iter().find(|r| r.label == row.label)
        && !units_agree(&aligned.cells)
    {
        return None;
    }
    values.iter().any(|v| *v > 0.0).then_some(values)
}

fn format_score(lang: Lang, score: f32) -> String {
    let s = format!("{score:.1}");
    match lang {
        Lang::Es => s.replace('.', ","),
        Lang::En => s,
    }
}

pub(super) fn section_label<'a>(label: &str) -> Element<'a, Message> {
    text(label.to_uppercase())
        .size(11)
        .style(theme::text_muted)
        .into()
}

pub(super) fn divider<'a>() -> Element<'a, Message> {
    rule::horizontal(1)
        .style(|theme: &Theme| rule::Style {
            color: tokens(theme::is_dark(theme)).border,
            radius: 0.0.into(),
            fill_mode: rule::FillMode::Full,
            snap: true,
        })
        .into()
}

fn panel(theme: &Theme) -> container::Style {
    let t = tokens(theme::is_dark(theme));
    container::Style {
        text_color: Some(t.foreground),
        background: Some(Background::Color(t.muted)),
        border: Border {
            color: t.border,
            width: 1.0,
            radius: theme::RADIUS.into(),
        },
        ..container::Style::default()
    }
}

struct Bar {
    fraction: f32,
    fill: Color,
    track: Color,
    outline: Color,
}

impl canvas::Program<Message> for Bar {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let radius = (theme::RADIUS / 2.0).into();
        frame.fill(
            &canvas::Path::rounded_rectangle(Point::ORIGIN, bounds.size(), radius),
            self.track,
        );
        let width = (bounds.width * self.fraction).max(3.0).min(bounds.width);
        let bar = canvas::Path::rounded_rectangle(
            Point::new(0.5, 0.5),
            Size::new((width - 1.0).max(0.0), (bounds.height - 1.0).max(0.0)),
            radius,
        );
        frame.fill(&bar, self.fill);
        frame.stroke(
            &bar,
            canvas::Stroke::default()
                .with_color(self.outline)
                .with_width(1.0),
        );
        vec![frame.into_geometry()]
    }
}

async fn save_record(store: Arc<Store>, record: ComparisonRecord) -> Result<i64, String> {
    store
        .save_comparison(&record)
        .await
        .map_err(|e| e.to_string())
}

impl App {
    pub(super) fn toggle_debate(&mut self) {
        self.debate_open = !self.debate_open;
    }

    pub(super) fn rerun(&mut self) -> Task<Message> {
        if self.result.is_none() {
            return Task::none();
        }
        self.compare()
    }

    /// The pipeline input with the products scraped during the run filled in,
    /// so a stored comparison can be re-run without scraping again.
    fn history_record(&self, result: &ComparisonResult) -> Option<ComparisonRecord> {
        let mut input = self.input.clone()?;
        for (k, item) in input.items.iter_mut().enumerate() {
            let status = self.run_items.get(k).map(|&i| &self.items[i].status);
            if let Some(ItemStatus::Ready(p)) = status {
                item.product = Some((**p).clone());
            }
        }
        Some(ComparisonRecord {
            input,
            result: result.clone(),
        })
    }

    pub(super) fn finish_run(&mut self, result: ComparisonResult) -> Task<Message> {
        let record = self.history_record(&result);
        if let Some(record) = &record {
            self.input = Some(record.input.clone());
        }
        self.result = Some(result);
        self.stale = false;
        self.run = super::RunState::Idle;
        match (self.store.clone(), record) {
            (Some(store), Some(record)) => {
                Task::perform(save_record(store, record), Message::HistorySaved)
            }
            _ => Task::none(),
        }
    }

    pub(super) fn results_card(&self) -> Option<Element<'_, Message>> {
        let result = self.result.as_ref()?;
        let mut page = column![].spacing(16);
        if let Some(banner) = self.excluded_banner(result) {
            page = page.push(banner);
        }

        let mut card =
            column![self.verdict(result), divider(), self.stats_table(result)].spacing(20);
        if let Some(charts) = self.charts(result) {
            card = card.push(divider()).push(charts);
        }
        if !result.advocates.is_empty() {
            card = card.push(divider()).push(self.debate(result));
        }
        page = page.push(
            container(card)
                .padding(24)
                .width(Length::Fill)
                .style(theme::card),
        );
        Some(page.into())
    }

    fn product_title(result: &ComparisonResult, i: usize) -> String {
        result
            .products
            .get(i)
            .map_or_else(String::new, |p| short(&p.title, TITLE_CHARS))
    }

    fn excluded_banner<'a>(&'a self, result: &'a ComparisonResult) -> Option<Element<'a, Message>> {
        if result.excluded.is_empty() {
            return None;
        }
        let lang = self.settings.lang;
        let mut lines =
            column![text(t(lang, Key::ExcludedBanner)).size(14).font(bold())].spacing(4);
        for (idx, err) in &result.excluded {
            let name = self
                .input
                .as_ref()
                .and_then(|input| input.items.get(*idx))
                .map(|item| {
                    let url = item.url.split_once("://").map_or(&*item.url, |(_, r)| r);
                    short(url, 48)
                })
                .unwrap_or_else(|| format!("#{}", idx + 1));
            lines = lines.push(text(format!("{name}: {}", err.message)).size(13));
        }
        Some(
            container(lines)
                .padding([12, 16])
                .width(Length::Fill)
                .style(chip_style(Tone::Bad))
                .into(),
        )
    }

    fn verdict<'a>(&'a self, result: &'a ComparisonResult) -> Element<'a, Message> {
        let lang = self.settings.lang;
        let t_ = tokens(theme::is_dark(&self.theme()));
        let judge = &result.judge;

        let mut head = row![section_label(t(lang, Key::ResultVerdict))].align_y(Alignment::Center);
        head = head.push(Space::new().width(Length::Fill));
        if self.stale {
            head = head
                .push(
                    container(text(t(lang, Key::Stale)).size(12))
                        .padding([4, 10])
                        .style(chip_style(Tone::Muted)),
                )
                .push(
                    button(text(t(lang, Key::Rerun)).size(13))
                        .padding([6, 12])
                        .style(theme::button_secondary)
                        .on_press_maybe((!self.running()).then_some(Message::Rerun)),
                )
                .spacing(8);
        }

        let mut col = column![head].spacing(8);
        if let Some(winner) = result.products.get(judge.winner) {
            col = col.push(text(&winner.title).size(20).font(bold()).color(t_.primary));
        }
        col = col.push(text(&judge.verdict).size(14));

        let mut scores = row![].spacing(20);
        for (i, score) in judge.scores.iter().enumerate() {
            let winner = i == judge.winner;
            let value = text(format_score(lang, *score)).size(13);
            let value = if winner {
                value.font(bold()).color(t_.primary)
            } else {
                value.font(bold())
            };
            scores = scores.push(
                row![
                    text(Self::product_title(result, i))
                        .size(13)
                        .style(theme::text_muted),
                    value
                ]
                .spacing(8),
            );
        }
        col.push(scores.wrap()).into()
    }

    fn ai_marker<'a>(&self) -> Element<'a, Message> {
        let lang = self.settings.lang;
        let badge = match lang {
            Lang::Es => "IA",
            Lang::En => "AI",
        };
        tooltip(
            container(text(badge).size(10))
                .padding([1, 5])
                .style(chip_style(Tone::Muted)),
            container(text(t(lang, Key::AiDerived)).size(12))
                .padding([6, 10])
                .style(theme::card),
            Position::Top,
        )
        .gap(4)
        .into()
    }

    fn stat_cell<'a>(
        &self,
        result: &ComparisonResult,
        row: &StatRow,
        i: usize,
    ) -> Element<'a, Message> {
        let lang = self.settings.lang;
        let t_ = tokens(theme::is_dark(&self.theme()));
        let Some(value) = cell_text(result, row, i) else {
            return text("\u{2014}").size(13).style(theme::text_muted).into();
        };
        let label = text(value).size(13);
        let label = if row.best.contains(&i) {
            label.font(bold()).color(t_.primary)
        } else {
            label
        };

        let mut line = row![label].spacing(6).align_y(Alignment::Center);
        if row.cells[i].ai_derived {
            line = line.push(self.ai_marker());
        }
        let unconfirmed = row.kind == RowKind::Total
            && result
                .costs
                .get(i)
                .is_some_and(|c| c.shipping == ShippingCost::Unknown);
        if unconfirmed {
            column![
                line,
                text(format!(
                    "{} {}",
                    t(lang, Key::CostShipping),
                    t(lang, Key::ShippingToConfirm)
                ))
                .size(11)
                .style(theme::text_muted)
            ]
            .spacing(2)
            .into()
        } else {
            line.into()
        }
    }

    fn stats_table<'a>(&'a self, result: &'a ComparisonResult) -> Element<'a, Message> {
        let lang = self.settings.lang;
        let t_ = tokens(theme::is_dark(&self.theme()));
        let label_col = Length::FillPortion(2);
        let value_col = Length::FillPortion(3);

        let mut header = row![Space::new().width(label_col)]
            .spacing(12)
            .align_y(Alignment::End);
        for (i, product) in result.products.iter().enumerate() {
            let title = text(short(&product.title, TITLE_CHARS))
                .size(13)
                .font(bold());
            let title = if i == result.judge.winner {
                title.color(t_.primary)
            } else {
                title
            };
            header = header.push(
                container(
                    column![
                        title,
                        container(text(site_label(product.site)).size(11))
                            .padding([2, 8])
                            .style(chip_style(Tone::Muted)),
                    ]
                    .spacing(4),
                )
                .width(value_col),
            );
        }

        let mut table = column![
            section_label(t(lang, Key::ResultStats)),
            container(header).padding([0, 0]),
            divider()
        ]
        .spacing(8);
        for (n, stat) in result.stats.iter().enumerate() {
            let mut line = row![
                container(
                    text(row_label(lang, stat))
                        .size(13)
                        .style(theme::text_muted)
                )
                .width(label_col)
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            for i in 0..result.products.len() {
                line = line.push(container(self.stat_cell(result, stat, i)).width(value_col));
            }
            table = table.push(container(line).padding([2, 0]));
            if n + 1 < result.stats.len() {
                table = table.push(divider());
            }
        }
        table.into()
    }

    fn charts<'a>(&'a self, result: &'a ComparisonResult) -> Option<Element<'a, Message>> {
        let lang = self.settings.lang;
        let t_ = tokens(theme::is_dark(&self.theme()));
        let judge_rows = &result.judge.rows;

        let mut groups = column![].spacing(18);
        let mut any = false;
        for stat in &result.stats {
            let Some(values) = bar_values(stat, judge_rows) else {
                continue;
            };
            any = true;
            let max = values.iter().copied().fold(0.0, f64::max);

            let mut head = row![text(row_label(lang, stat)).size(13).font(bold())]
                .spacing(8)
                .align_y(Alignment::Center);
            if better(stat, judge_rows) == Better::Lower {
                head = head.push(
                    text(format!("({})", t(lang, Key::LowerIsBetter)))
                        .size(11)
                        .style(theme::text_muted),
                );
            }

            let mut group = column![head].spacing(6);
            for (i, value) in values.iter().enumerate() {
                let best = stat.best.contains(&i);
                let shown = text(cell_text(result, stat, i).unwrap_or_default()).size(13);
                let shown = if best {
                    shown.font(bold()).color(t_.primary)
                } else {
                    shown
                };
                let bar = Bar {
                    fraction: (*value / max) as f32,
                    fill: t_.chart[i % t_.chart.len()],
                    track: t_.muted,
                    outline: Color {
                        a: 0.6,
                        ..t_.muted_foreground
                    },
                };
                group = group.push(
                    row![
                        container(
                            text(Self::product_title(result, i))
                                .size(12)
                                .style(theme::text_muted)
                        )
                        .width(BAR_LABEL_WIDTH),
                        canvas(bar).width(Length::Fill).height(BAR_HEIGHT),
                        container(shown).width(BAR_VALUE_WIDTH),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                );
            }
            groups = groups.push(group);
        }
        any.then(|| {
            column![section_label(t(lang, Key::ResultCharts)), groups]
                .spacing(12)
                .into()
        })
    }

    fn debate<'a>(&'a self, result: &'a ComparisonResult) -> Element<'a, Message> {
        let lang = self.settings.lang;
        let arrow = if self.debate_open {
            "\u{25BE}"
        } else {
            "\u{25B8}"
        };
        let toggle = button(
            row![
                text(arrow).size(15).shaping(text::Shaping::Advanced),
                text(t(lang, Key::ResultDebate).to_uppercase()).size(11)
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding([4, 0])
        .style(theme::button_ghost)
        .on_press(Message::ToggleDebate);

        let mut col = column![toggle].spacing(12);
        if !self.debate_open {
            return col.into();
        }
        for advocate in &result.advocates {
            let mut body = column![
                text(Self::product_title(result, advocate.product))
                    .size(14)
                    .font(bold()),
                text(&advocate.argument).size(13),
            ]
            .spacing(6);
            if !advocate.strengths.is_empty() {
                body = body.push(
                    text(t(lang, Key::DebateStrengths))
                        .size(12)
                        .style(theme::text_muted),
                );
                for s in &advocate.strengths {
                    body = body.push(text(format!("\u{2022} {s}")).size(13));
                }
            }
            for attack in &advocate.attacks {
                body = body.push(
                    text(format!(
                        "{} {}: {}",
                        t(lang, Key::DebateAttacks),
                        Self::product_title(result, attack.target),
                        attack.point
                    ))
                    .size(13)
                    .style(theme::text_muted),
                );
            }
            col = col.push(container(body).padding(16).width(Length::Fill).style(panel));
        }
        col.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::demo_comparison;
    use crate::domain::FxRate;
    use crate::pipeline::PipelineEvent;
    use crate::settings::Settings;
    use crate::ui::RunState;
    use crate::ui::snap::DevEnv;

    fn demo_app() -> App {
        let dev = DevEnv {
            demo: true,
            ..DevEnv::default()
        };
        let mut app = App::new(Settings::default(), None, dev);
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

    fn pipeline(app: &mut App, event: PipelineEvent) -> Task<Message> {
        app.update(Message::Pipeline(Box::new(event)))
    }

    #[test]
    fn toggle_debate_flips() {
        let mut app = demo_app();
        assert!(!app.debate_open);
        let _ = app.update(Message::ToggleDebate);
        assert!(app.debate_open);
        let _ = app.update(Message::ToggleDebate);
        assert!(!app.debate_open);
    }

    #[test]
    fn rerun_reuses_scraped_products() {
        let mut app = demo_app();
        app.stale = true;
        let task = app.update(Message::Rerun);
        assert_eq!(task.units(), 1);
        assert!(matches!(app.run, RunState::Running { .. }));

        let _ = app.update(Message::FxResolved(Ok(fx())));
        let input = app.input.as_ref().unwrap();
        assert_eq!(input.items.len(), 2);
        assert!(input.items.iter().all(|i| i.product.is_some()));
    }

    #[test]
    fn rerun_without_result_is_a_noop() {
        let mut app = demo_app();
        app.result = None;
        assert_eq!(app.update(Message::Rerun).units(), 0);
        assert_eq!(app.run, RunState::Idle);
    }

    #[test]
    fn rerun_without_key_reports_it() {
        let mut app = demo_app();
        app.set_api_key(None);
        assert_eq!(app.update(Message::Rerun).units(), 0);
        assert_eq!(app.run, RunState::Error(crate::ui::RunError::NoApiKey));
    }

    #[test]
    fn finishing_without_a_store_saves_nothing() {
        let mut app = demo_app();
        app.stale = true;
        let _ = app.update(Message::Compare);
        let _ = app.update(Message::FxResolved(Ok(fx())));
        let task = pipeline(&mut app, PipelineEvent::Finished(Ok(demo_comparison().1)));
        assert_eq!(task.units(), 0);
        assert!(!app.stale);
        assert_eq!(app.run, RunState::Idle);
    }

    #[tokio::test]
    async fn finishing_saves_a_new_history_entry() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("t.db")).await.unwrap());
        let dev = DevEnv {
            demo: true,
            ..DevEnv::default()
        };
        let mut app = App::new(Settings::default(), Some(Arc::clone(&store)), dev);
        app.set_api_key(Some("test-key".into()));
        app.stale = true;
        let _ = app.update(Message::Compare);
        let _ = app.update(Message::FxResolved(Ok(fx())));

        let result = demo_comparison().1;
        let record = app.history_record(&result).unwrap();
        let task = pipeline(&mut app, PipelineEvent::Finished(Ok(result.clone())));
        assert_eq!(task.units(), 1);
        assert!(!app.stale);
        assert_eq!(app.result, Some(result));

        save_record(Arc::clone(&store), record.clone())
            .await
            .unwrap();
        save_record(Arc::clone(&store), record).await.unwrap();
        assert_eq!(store.list_history(10).await.unwrap().len(), 2);
    }

    #[test]
    fn stored_input_keeps_products_scraped_during_the_run() {
        let mut app = demo_app();
        app.items[0].status = ItemStatus::Idle;
        app.items[1].status = ItemStatus::Idle;
        let _ = app.update(Message::Compare);
        let _ = app.update(Message::FxResolved(Ok(fx())));
        let products = demo_comparison().1.products;
        for (item, p) in products.iter().enumerate() {
            let _ = pipeline(
                &mut app,
                PipelineEvent::Scraped {
                    item,
                    result: Ok(p.clone()),
                },
            );
        }
        let result = demo_comparison().1;
        let record = app.history_record(&result).unwrap();
        assert!(record.input.items.iter().all(|i| i.product.is_some()));
    }

    #[test]
    fn converted_totals_carry_an_approx_marker() {
        let result = demo_comparison().1;
        let total = &result.stats[0];
        assert!(
            cell_text(&result, total, 0)
                .unwrap()
                .starts_with('\u{2248}')
        );
        assert!(
            !cell_text(&result, total, 1)
                .unwrap()
                .starts_with('\u{2248}')
        );
    }

    #[test]
    fn missing_cells_have_no_text() {
        let mut result = demo_comparison().1;
        result.stats[1].cells[0].raw = None;
        assert_eq!(cell_text(&result, &result.stats[1], 0), None);
        assert!(cell_text(&result, &result.stats[1], 1).is_some());
    }

    #[test]
    fn bars_need_numbers_for_every_product() {
        let result = demo_comparison().1;
        let rows = &result.judge.rows;
        let ram = result.stats.iter().find(|r| r.label == "RAM").unwrap();
        assert_eq!(bar_values(ram, rows), Some(vec![16.0, 12.0]));

        let mut missing = ram.clone();
        missing.cells[1].numeric = None;
        assert_eq!(bar_values(&missing, rows), None);
    }

    #[test]
    fn bars_skip_rows_with_mismatched_units() {
        let result = demo_comparison().1;
        let mut rows = result.judge.rows.clone();
        rows[0].cells[1].unit = Some("MB".into());
        let ram = result.stats.iter().find(|r| r.label == "RAM").unwrap();
        assert_eq!(bar_values(ram, &rows), None);
    }

    #[test]
    fn direction_comes_from_row_kind_or_judge() {
        let result = demo_comparison().1;
        let rows = &result.judge.rows;
        assert_eq!(better(&result.stats[0], rows), Better::Lower);
        assert_eq!(better(&result.stats[1], rows), Better::Higher);
        let ram = result.stats.iter().find(|r| r.label == "RAM").unwrap();
        assert_eq!(better(ram, rows), Better::Higher);
    }

    #[test]
    fn shortens_long_titles() {
        assert_eq!(short("abc", 5), "abc");
        assert_eq!(short("abcdefgh", 5), "abcd\u{2026}");
    }

    #[test]
    fn scores_follow_language_decimal() {
        assert_eq!(format_score(Lang::Es, 8.4), "8,4");
        assert_eq!(format_score(Lang::En, 8.4), "8.4");
    }

    #[test]
    fn results_card_renders_only_with_a_result() {
        let mut app = demo_app();
        assert!(app.results_card().is_some());
        app.result = None;
        assert!(app.results_card().is_none());
    }
}
