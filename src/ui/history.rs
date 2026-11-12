//! History screen: stored comparisons reopened from the local store, offline.

use std::str::FromStr;
use std::sync::Arc;

use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Length, Task};

use super::input::{Tone, chip_style};
use super::results::{bold, short};
use super::{App, Message, RunState, Screen};
use crate::i18n::{Key, Lang, t};
use crate::store::{ComparisonRecord, HistoryEntry, Store};
use crate::theme::{self, tokens};

const HISTORY_LIMIT: u32 = 200;

fn format_date(raw: &str) -> String {
    jiff::Timestamp::from_str(raw)
        .map(|ts| {
            ts.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| raw.to_owned())
}

fn preset_label(lang: Lang, raw: &str) -> String {
    let key = match raw {
        "best_overall" => Key::PresetBestOverall,
        "quality_price" => Key::PresetBestValue,
        "cheapest_decent" => Key::PresetCheapest,
        "custom" => Key::PresetCustom,
        other => return other.to_owned(),
    };
    t(lang, key).to_owned()
}

async fn load_list(store: Arc<Store>) -> Vec<HistoryEntry> {
    store.list_history(HISTORY_LIMIT).await.unwrap_or_else(|e| {
        eprintln!("maeve: cannot load history: {e}");
        Vec::new()
    })
}

pub(super) async fn load_record(
    store: Arc<Store>,
    id: i64,
) -> Result<Box<ComparisonRecord>, String> {
    store
        .load_comparison(id)
        .await
        .map(Box::new)
        .map_err(|e| e.to_string())
}

async fn delete_record(store: Arc<Store>, id: i64) -> Result<i64, String> {
    store
        .delete_comparison(id)
        .await
        .map(|()| id)
        .map_err(|e| e.to_string())
}

impl App {
    pub(super) fn load_history(&self) -> Task<Message> {
        match self.store.clone() {
            Some(store) => Task::perform(load_list(store), Message::HistoryLoaded),
            None => Task::none(),
        }
    }

    pub(super) fn open_history(&mut self, id: i64) -> Task<Message> {
        if self.running() || self.extracting() {
            return Task::none();
        }
        self.history_error = false;
        match self.store.clone() {
            Some(store) => Task::perform(load_record(store, id), Message::HistoryOpened),
            None => Task::none(),
        }
    }

    /// Restores input and result from the store; never starts a pipeline run.
    pub(super) fn history_opened(
        &mut self,
        record: Result<Box<ComparisonRecord>, String>,
    ) -> Task<Message> {
        if self.running() || self.extracting() {
            return Task::none();
        }
        let ComparisonRecord { input, result } = match record {
            Ok(record) => *record,
            Err(e) => {
                eprintln!("maeve: cannot open comparison: {e}");
                self.history_error = true;
                return Task::none();
            }
        };
        self.load_input(&input);
        self.stale = input.cost != self.settings.cost;
        self.input = Some(input);
        self.result = Some(result);
        self.run = RunState::Idle;
        self.run_items.clear();
        self.paste_row = None;
        self.debate_open = false;
        self.history_confirm = None;
        self.screen = Screen::Compare;
        Task::none()
    }

    pub(super) fn delete_history(&mut self, id: i64) -> Task<Message> {
        self.history_confirm = None;
        match self.store.clone() {
            Some(store) => Task::perform(delete_record(store, id), Message::HistoryDeleted),
            None => Task::none(),
        }
    }

    pub(super) fn history_deleted(&mut self, result: Result<i64, String>) -> Task<Message> {
        match result {
            Ok(id) => self.history.retain(|e| e.id != id),
            Err(e) => eprintln!("maeve: cannot delete comparison: {e}"),
        }
        Task::none()
    }

    fn history_row<'a>(&'a self, entry: &'a HistoryEntry) -> Element<'a, Message> {
        let lang = self.settings.lang;
        let t_ = tokens(theme::is_dark(&self.theme()));

        let titles = entry
            .titles
            .iter()
            .map(|title| short(title, 40))
            .collect::<Vec<_>>()
            .join("  vs  ");
        let info = column![
            row![
                text(format_date(&entry.created_at))
                    .size(12)
                    .style(theme::text_muted),
                container(text(preset_label(lang, &entry.preset)).size(12))
                    .padding([4, 8])
                    .style(chip_style(Tone::Muted)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            text(titles).size(14),
            text(&entry.winner).size(14).font(bold()).color(t_.primary),
        ]
        .spacing(4)
        .width(Length::Fill);

        let actions: Element<'_, Message> = if self.history_confirm == Some(entry.id) {
            row![
                text(t(lang, Key::HistoryDeleteConfirm)).size(13),
                button(text(t(lang, Key::HistoryDeleteYes)).size(13))
                    .padding([8, 12])
                    .style(theme::button_destructive)
                    .on_press(Message::DeleteHistory(entry.id)),
                button(text(t(lang, Key::Cancel)).size(13))
                    .padding([8, 12])
                    .style(theme::button_secondary)
                    .on_press(Message::CancelDeleteHistory),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        } else {
            let can_open = !self.running() && !self.extracting();
            row![
                button(text(t(lang, Key::HistoryOpen)).size(13))
                    .padding([8, 12])
                    .style(theme::button_secondary)
                    .on_press_maybe(can_open.then_some(Message::OpenHistory(entry.id))),
                button(text(t(lang, Key::HistoryDelete)).size(13))
                    .padding([8, 12])
                    .style(theme::button_ghost)
                    .on_press(Message::AskDeleteHistory(entry.id)),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        };

        container(row![info, actions].spacing(16).align_y(Alignment::Center))
            .padding([16, 16])
            .width(Length::Fill)
            .style(theme::list_item)
            .into()
    }

    pub(super) fn history_view(&self) -> Element<'_, Message> {
        let lang = self.settings.lang;
        let t_ = tokens(theme::is_dark(&self.theme()));

        let mut col = column![text(t(lang, Key::NavHistory)).size(22)].spacing(16);
        if self.history_error {
            col = col.push(
                text(t(lang, Key::HistoryOpenFailed))
                    .size(13)
                    .color(t_.destructive),
            );
        }
        if self.history.is_empty() {
            col = col.push(
                text(t(lang, Key::HistoryEmpty))
                    .size(14)
                    .style(theme::text_muted),
            );
        } else {
            let mut list = column![].spacing(8);
            for entry in &self.history {
                list = list.push(self.history_row(entry));
            }
            col = col.push(list);
        }

        container(col)
            .padding(24)
            .width(Length::Fill)
            .style(theme::card)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::demo_comparison;
    use crate::settings::Settings;
    use crate::ui::ItemStatus;
    use crate::ui::snap::DevEnv;

    async fn store_with_record() -> (tempfile::TempDir, Arc<Store>, i64, ComparisonRecord) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("t.db")).await.unwrap());
        let (input, result) = demo_comparison();
        let record = ComparisonRecord { input, result };
        let id = store.save_comparison(&record).await.unwrap();
        (dir, store, id, record)
    }

    fn app_with(store: Arc<Store>) -> App {
        App::new(Settings::default(), Some(store), DevEnv::default())
    }

    #[tokio::test]
    async fn opening_restores_input_and_result_and_starts_no_run() {
        let (_dir, store, id, record) = store_with_record().await;
        let mut app = app_with(Arc::clone(&store));
        app.screen = Screen::History;

        let task = app.update(Message::OpenHistory(id));
        assert_eq!(task.units(), 1);
        assert!(app.result.is_none());

        let loaded = load_record(store, id).await.unwrap();
        let task = app.update(Message::HistoryOpened(Ok(loaded)));
        assert_eq!(task.units(), 0);
        assert_eq!(app.result.as_ref(), Some(&record.result));
        assert_eq!(app.input.as_ref(), Some(&record.input));
        assert_eq!(app.run, RunState::Idle);
        assert_eq!(app.screen, Screen::Compare);
        assert!(!app.stale);
        assert_eq!(app.items.len(), 2);
        assert_eq!(app.items[0].url, record.input.items[0].url);
        assert!(
            app.items
                .iter()
                .all(|i| matches!(i.status, ItemStatus::Ready(_)))
        );
        assert_eq!(app.purpose, "Trabajo de oficina y programación");
        assert_eq!(app.preset, record.input.preset);
    }

    #[tokio::test]
    async fn restored_comparison_with_other_cost_settings_is_stale() {
        let (_dir, store, id, _) = store_with_record().await;
        let mut settings = Settings::default();
        settings.cost.iva_bp = 1000;
        let mut app = App::new(settings, Some(Arc::clone(&store)), DevEnv::default());
        let loaded = load_record(store, id).await.unwrap();
        let _ = app.update(Message::HistoryOpened(Ok(loaded)));
        assert!(app.stale);
    }

    #[tokio::test]
    async fn opening_is_ignored_while_a_run_is_in_flight() {
        let (_dir, store, id, record) = store_with_record().await;
        let mut app = app_with(store);
        app.run = RunState::Running {
            advocates_done: 0,
            waiting: None,
            judging: false,
        };
        assert_eq!(app.update(Message::OpenHistory(id)).units(), 0);
        let task = app.update(Message::HistoryOpened(Ok(Box::new(record))));
        assert_eq!(task.units(), 0);
        assert!(app.result.is_none());
    }

    #[test]
    fn open_failure_is_reported_on_the_history_screen() {
        let mut app = App::new(Settings::default(), None, DevEnv::default());
        let _ = app.update(Message::HistoryOpened(Err("gone".into())));
        assert!(app.history_error);
        assert!(app.result.is_none());
    }

    #[tokio::test]
    async fn delete_needs_confirmation_then_removes_the_row() {
        let (_dir, store, id, _) = store_with_record().await;
        let mut app = app_with(Arc::clone(&store));
        let list = load_list(Arc::clone(&store)).await;
        let _ = app.update(Message::HistoryLoaded(list));
        assert_eq!(app.history.len(), 1);

        let _ = app.update(Message::AskDeleteHistory(id));
        assert_eq!(app.history_confirm, Some(id));
        let _ = app.update(Message::CancelDeleteHistory);
        assert_eq!(app.history_confirm, None);
        assert_eq!(app.history.len(), 1);

        let _ = app.update(Message::AskDeleteHistory(id));
        assert_eq!(app.update(Message::DeleteHistory(id)).units(), 1);
        assert_eq!(app.history_confirm, None);
        let deleted = delete_record(Arc::clone(&store), id).await;
        let _ = app.update(Message::HistoryDeleted(deleted));
        assert!(app.history.is_empty());
        assert!(store.list_history(10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn navigating_to_history_reloads_the_list() {
        let (_dir, store, _, _) = store_with_record().await;
        let mut app = app_with(store);
        assert_eq!(app.update(Message::Navigate(Screen::History)).units(), 1);
        assert_eq!(app.update(Message::Navigate(Screen::Settings)).units(), 0);
    }

    #[test]
    fn dates_are_shown_locally_and_fall_back_to_raw() {
        let shown = format_date("2026-11-12T14:05:00Z");
        assert!(shown.starts_with("2026-11-1"), "{shown}");
        assert_eq!(shown.len(), 16);
        assert_eq!(format_date("not a date"), "not a date");
    }

    #[test]
    fn preset_labels_are_localized_and_custom_names_pass_through() {
        assert_eq!(preset_label(Lang::En, "best_overall"), "Best overall");
        assert_eq!(
            preset_label(Lang::Es, "cheapest_decent"),
            "Más barato decente"
        );
        assert_eq!(preset_label(Lang::Es, "Para la oficina"), "Para la oficina");
    }
}
