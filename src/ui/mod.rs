mod browser;
mod header;
mod history;
mod input;
mod loading;
mod paste;
mod results;
mod settings;
pub mod snap;

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::theme::Mode;
use iced::widget::{column, container, operation, scrollable};
use iced::{Element, Event, Length, Subscription, Task, Theme, event, system, window};

use crate::demo::demo_comparison;
use crate::domain::{FxRate, Product};
use crate::i18n::{Key, Lang};
use crate::llm::GeminiClient;
use crate::llm::prompts::Preset;
use crate::pipeline::{CompareInput, ComparisonResult, PipelineEvent};
use crate::scrape::{ScrapeError, SidecarCommand, SidecarScraper};
use crate::settings::{Settings, ThemePref, load_api_key};
use crate::store::{ComparisonRecord, HistoryEntry, Store, default_db_path};
use crate::theme;
use settings::{SettingsDraft, SettingsField};
use snap::DevEnv;

const PAGE_SCROLL: &str = "page";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Compare,
    History,
    Settings,
}

#[derive(Debug, Clone)]
pub enum ItemStatus {
    Idle,
    Scraping,
    Ready(Box<Product>),
    Failed(ScrapeError),
    Extracting,
}

#[derive(Debug, Clone)]
pub struct ItemState {
    pub url: String,
    pub status: ItemStatus,
    /// What the user typed; only meaningful when `shipping_user_set`. Otherwise
    /// the box shows the cost engine's default, recomputed on every view.
    pub shipping_input: String,
    pub shipping_user_set: bool,
    pub shipping_invalid: bool,
    /// Problem with the last paste attempt; the item itself stays `Failed`.
    pub paste_error: Option<String>,
}

impl ItemState {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            status: ItemStatus::Idle,
            shipping_input: String::new(),
            shipping_user_set: false,
            shipping_invalid: false,
            paste_error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    NoApiKey,
    NotEnoughProducts,
    Auth,
    Overloaded,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RunState {
    #[default]
    Idle,
    Running {
        advocates_done: usize,
        waiting: Option<Duration>,
        judging: bool,
    },
    Error(RunError),
}

struct LlmHandle {
    client: Arc<GeminiClient>,
    key: String,
    rpm: u32,
}

#[derive(Clone)]
pub struct StoreHandle(Arc<Store>);

impl fmt::Debug for StoreHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StoreHandle")
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    SystemTheme(Mode),
    SetThemePref(ThemePref),
    SetLang(Lang),
    Navigate(Screen),
    Booted(Option<StoreHandle>, Box<Settings>),
    Saved(Result<(), String>),
    ApiKeyLoaded(Option<String>),
    PresetsLoaded(Vec<Preset>),
    UrlChanged(usize, String),
    AddItem,
    RemoveItem(usize),
    PresetPicked(Preset),
    CriteriaChanged(String),
    PurposeChanged(String),
    ShippingChanged(usize, String),
    Compare,
    FxResolved(Result<FxRate, String>),
    Pipeline(Box<PipelineEvent>),
    Retry(usize),
    PasteMode(usize),
    PasteText(usize),
    PasteImage(usize),
    FileDropped(PathBuf),
    PastedText(usize, Option<String>),
    PastedImage(usize, Result<paste::PastedImage, Key>),
    Extracted(usize, Result<Box<Product>, String>),
    OpenInBrowser(usize),
    Tick(Instant),
    ModelsLoaded(Result<Vec<String>, String>),
    ModelPicked(SettingsField, settings::ModelOption),
    BrowserPage(Box<crate::bridge::Page>),
    ToggleDebate,
    Rerun,
    HistorySaved(Result<i64, String>),
    HistoryLoaded(Vec<HistoryEntry>),
    OpenHistory(i64),
    HistoryOpened(Result<Box<ComparisonRecord>, String>),
    AskDeleteHistory(i64),
    CancelDeleteHistory,
    DeleteHistory(i64),
    HistoryDeleted(Result<i64, String>),
    SettingsInput(SettingsField, String),
    SettingsChanged(Box<Settings>),
    SaveApiKey(String),
    ApiKeySaved(Result<String, Key>),
    EditApiKey(bool),
    PresetNameChanged(String),
    SavePreset(Preset),
    PresetSaved(Result<Preset, String>),
    ScrollSnapshot(f32),
    TakeSnapshot,
    Snapshot(window::Screenshot),
}

pub struct App {
    pub settings: Settings,
    pub system_mode: Mode,
    pub screen: Screen,
    pub input: Option<CompareInput>,
    pub result: Option<ComparisonResult>,
    pub stale: bool,
    pub debate_open: bool,
    pub items: Vec<ItemState>,
    pub run: RunState,
    pub preset: Preset,
    pub purpose: String,
    pub custom_presets: Vec<Preset>,
    pub fx: Option<FxRate>,
    /// Failed row that receives dropped image files.
    pub paste_row: Option<usize>,
    pub preset_name: String,
    pub history: Vec<HistoryEntry>,
    pub history_confirm: Option<i64>,
    pub history_error: bool,
    pub draft: SettingsDraft,
    pub key_notice: Option<Key>,
    /// Verifying and saving a new key; disables the form.
    pub key_busy: bool,
    /// Shows the key input even though a key is configured.
    pub key_editing: bool,
    /// Animation clock: frames come from `Message::Tick` while something loads.
    now: Instant,
    anim_start: Instant,
    phrase_seed: usize,
    /// Models offered in Settings; filled from the API once a key exists.
    models: Vec<String>,
    models_requested: bool,
    /// Advocate / judge model picker is on "Custom…".
    custom_model: [bool; 2],
    api_key: Option<String>,
    run_items: Vec<usize>,
    scraper: Arc<SidecarScraper>,
    llm: Option<LlmHandle>,
    store: Option<Arc<Store>>,
    dev: DevEnv,
}

fn scraper_command() -> SidecarCommand {
    use std::path::{Path, PathBuf};

    if let Some(dir) = std::env::var_os("MAEVE_SCRAPER_DIR") {
        return SidecarCommand::default_uv(dir.into());
    }
    // Installed bundles ship the scraper as a resource: Contents/Resources on
    // macOS, /usr/lib/maeve on Linux (cargo-bundle's deb layout).
    let bundled = std::env::current_exe().ok().and_then(|exe| {
        let bin_dir = exe.parent()?.to_path_buf();
        [
            bin_dir.join("../Resources/scraper"),
            bin_dir.join("../lib/maeve/scraper"),
        ]
        .into_iter()
        .find(|d| d.join("pyproject.toml").is_file())
    });
    match bundled {
        Some(dir) => {
            let state = dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("maeve");
            SidecarCommand::bundled_uv(dir, state)
        }
        None => SidecarCommand::default_uv(Path::new(env!("CARGO_MANIFEST_DIR")).join("scraper")),
    }
}

async fn load_key() -> Option<String> {
    match tokio::task::spawn_blocking(load_api_key).await {
        Ok(Ok(key)) => key,
        Ok(Err(e)) => {
            eprintln!("maeve: cannot read API key: {e}");
            None
        }
        Err(e) => {
            eprintln!("maeve: key lookup failed: {e}");
            None
        }
    }
}

async fn open_store() -> (Option<StoreHandle>, Box<Settings>) {
    let store = match Store::open(&default_db_path()).await {
        Ok(store) => store,
        Err(e) => {
            eprintln!("maeve: cannot open store: {e}");
            return (None, Box::default());
        }
    };
    let settings = store.load_settings().await.unwrap_or_else(|e| {
        eprintln!("maeve: cannot load settings: {e}");
        Settings::default()
    });
    (Some(StoreHandle(Arc::new(store))), Box::new(settings))
}

impl App {
    pub fn new(mut settings: Settings, store: Option<Arc<Store>>, dev: DevEnv) -> Self {
        dev.apply(&mut settings);
        let (input, result) = if dev.demo {
            let (input, result) = demo_comparison();
            (Some(input), Some(result))
        } else {
            (None, None)
        };
        let draft = SettingsDraft::from_settings(&settings);
        let mut app = Self {
            settings,
            system_mode: Mode::None,
            screen: dev.screen.unwrap_or_default(),
            items: vec![ItemState::new(""), ItemState::new("")],
            run: RunState::Idle,
            preset: Preset::BestOverall,
            purpose: String::new(),
            custom_presets: Vec::new(),
            fx: None,
            paste_row: None,
            preset_name: String::new(),
            history: Vec::new(),
            history_confirm: None,
            history_error: false,
            draft,
            key_notice: None,
            key_busy: false,
            key_editing: false,
            now: Instant::now(),
            anim_start: Instant::now(),
            phrase_seed: 0,
            models: Vec::new(),
            models_requested: false,
            custom_model: [false; 2],
            stale: false,
            debate_open: dev.debate,
            api_key: dev.gemini_key.clone(),
            run_items: Vec::new(),
            scraper: Arc::new(SidecarScraper::new(scraper_command())),
            llm: None,
            input: None,
            result,
            store,
            dev,
        };
        if let Some(input) = input {
            app.load_input(&input);
            app.input = Some(input);
        }
        if app.dev.failed {
            app.show_failed_row();
        }
        if app.dev.loading {
            app.show_loading();
        }
        if app.dev.demo {
            app.history = crate::demo::demo_history();
        }
        app.sync_llm();
        app
    }

    pub fn boot() -> (Self, Task<Message>) {
        let dev = DevEnv::from_env();
        // Demo mode never touches the real database: screenshots must not depend on
        // (or overwrite) the user's own settings and history.
        let booted = if dev.demo {
            Task::done(Message::Booted(None, Box::default()))
        } else {
            Task::perform(open_store(), |(store, settings)| {
                Message::Booted(store, settings)
            })
        };
        let mut tasks = vec![system::theme().map(Message::SystemTheme), booted];
        if dev.snapshot.is_none() && dev.gemini_key.is_none() {
            tasks.push(Task::perform(load_key(), Message::ApiKeyLoaded));
        }
        if dev.snapshot.is_some() {
            if let Some(y) = dev.scroll {
                tasks.push(Task::perform(
                    tokio::time::sleep(snap::SETTLE / 2),
                    move |()| Message::ScrollSnapshot(y),
                ));
            }
            tasks.push(Task::perform(tokio::time::sleep(snap::SETTLE), |()| {
                Message::TakeSnapshot
            }));
        }
        (
            Self::new(Settings::default(), None, dev),
            Task::batch(tasks),
        )
    }

    fn save(&self) -> Task<Message> {
        let Some(store) = self.store.clone() else {
            return Task::none();
        };
        let settings = self.settings.clone();
        Task::perform(
            async move {
                store
                    .save_settings(&settings)
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::Saved,
        )
    }

    pub fn update(&mut self, m: Message) -> Task<Message> {
        match m {
            Message::SystemTheme(mode) => {
                self.system_mode = mode;
                Task::none()
            }
            Message::SetThemePref(pref) => {
                self.settings.theme = pref;
                let save = self.save();
                if pref == ThemePref::System {
                    // Mode updates are only subscribed to while following the OS.
                    Task::batch([system::theme().map(Message::SystemTheme), save])
                } else {
                    save
                }
            }
            Message::SetLang(lang) => {
                self.settings.lang = lang;
                self.save()
            }
            Message::Navigate(screen) => {
                self.screen = screen;
                if screen == Screen::History {
                    self.history_confirm = None;
                    self.history_error = false;
                    return self.load_history();
                }
                if screen == Screen::Settings {
                    return self.load_models();
                }
                Task::none()
            }
            Message::Booted(store, mut settings) => {
                self.dev.apply(&mut settings);
                self.draft = SettingsDraft::from_settings(&settings);
                self.settings = *settings;
                self.store = store.map(|h| h.0);
                self.sync_llm();
                self.load_presets()
            }
            Message::ApiKeyLoaded(key) => {
                self.set_api_key(key);
                Task::none()
            }
            Message::PresetsLoaded(presets) => {
                self.custom_presets = presets;
                Task::none()
            }
            Message::UrlChanged(i, url) => {
                self.url_changed(i, url);
                Task::none()
            }
            Message::AddItem => {
                self.add_item();
                Task::none()
            }
            Message::RemoveItem(i) => {
                self.remove_item(i);
                Task::none()
            }
            Message::PresetPicked(preset) => {
                self.preset = preset;
                Task::none()
            }
            Message::CriteriaChanged(criteria) => {
                self.preset = input::ad_hoc_preset(criteria);
                Task::none()
            }
            Message::PurposeChanged(purpose) => {
                self.purpose = purpose;
                Task::none()
            }
            Message::ShippingChanged(i, value) => {
                self.shipping_changed(i, &value);
                Task::none()
            }
            Message::Compare => self.compare(),
            Message::FxResolved(fx) => self.fx_resolved(fx),
            Message::Pipeline(event) => self.on_pipeline(*event),
            Message::ToggleDebate => {
                self.toggle_debate();
                Task::none()
            }
            Message::Rerun => self.rerun(),
            Message::HistorySaved(result) => {
                if let Err(e) = result {
                    eprintln!("maeve: cannot save comparison: {e}");
                }
                Task::none()
            }
            Message::HistoryLoaded(entries) => {
                self.history = entries;
                Task::none()
            }
            Message::OpenHistory(id) => self.open_history(id),
            Message::HistoryOpened(record) => self.history_opened(record),
            Message::AskDeleteHistory(id) => {
                self.history_confirm = Some(id);
                Task::none()
            }
            Message::CancelDeleteHistory => {
                self.history_confirm = None;
                Task::none()
            }
            Message::DeleteHistory(id) => self.delete_history(id),
            Message::HistoryDeleted(result) => self.history_deleted(result),
            Message::SettingsInput(field, value) => self.settings_input(field, value),
            Message::SettingsChanged(settings) => self.apply_settings(*settings),
            Message::SaveApiKey(key) => self.save_key(key),
            Message::ApiKeySaved(result) => {
                self.key_saved(result);
                Task::none()
            }
            Message::EditApiKey(editing) => {
                self.edit_key(editing);
                Task::none()
            }
            Message::PresetNameChanged(name) => {
                self.preset_name = name;
                Task::none()
            }
            Message::SavePreset(preset) => self.save_preset(preset),
            Message::PresetSaved(result) => {
                self.preset_saved(result);
                Task::none()
            }
            Message::Retry(i) => self.retry(i),
            Message::PasteMode(i) => {
                self.toggle_paste_mode(i);
                Task::none()
            }
            Message::PasteText(i) => self.paste_text(i),
            Message::PasteImage(i) => self.paste_image(i),
            Message::FileDropped(path) => self.file_dropped(path),
            Message::PastedText(i, text) => self.pasted_text(i, text),
            Message::PastedImage(i, image) => self.pasted_image(i, image),
            Message::Extracted(i, result) => self.extracted(i, result),
            Message::OpenInBrowser(i) => {
                self.open_in_browser(i);
                Task::none()
            }
            Message::BrowserPage(page) => self.browser_page(*page),
            Message::ModelsLoaded(result) => {
                self.models_loaded(result);
                Task::none()
            }
            Message::ModelPicked(field, option) => self.model_picked(field, option),
            Message::Tick(now) => {
                self.now = now;
                Task::none()
            }
            Message::Saved(result) => {
                if let Err(e) = result {
                    eprintln!("maeve: cannot save settings: {e}");
                }
                Task::none()
            }
            Message::ScrollSnapshot(y) => {
                operation::snap_to(PAGE_SCROLL, scrollable::RelativeOffset { x: 0.0, y })
            }
            Message::TakeSnapshot => window::latest().then(|id| match id {
                Some(id) => window::screenshot(id).map(Message::Snapshot),
                None => iced::exit(),
            }),
            Message::Snapshot(shot) => {
                if let Some(path) = &self.dev.snapshot
                    && let Err(e) = snap::save_png(path, &shot)
                {
                    eprintln!("maeve: cannot save snapshot: {e}");
                }
                iced::exit()
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let lang = self.settings.lang;

        let page: Element<'_, Message> = match self.screen {
            Screen::Compare => {
                let mut page = column![self.input_card()].spacing(16);
                if let Some(loading) = self.loading_card() {
                    page = page.push(loading);
                } else if let Some(results) = self.results_card() {
                    page = page.push(results);
                }
                page.into()
            }
            Screen::History => self.history_view(),
            Screen::Settings => self.settings_view(),
        };

        column![
            header::view(lang, self.settings.theme, self.screen),
            scrollable(container(page).padding(24).width(Length::Fill))
                .id(PAGE_SCROLL)
                .height(Length::Fill),
        ]
        .into()
    }

    pub fn theme(&self) -> Theme {
        theme::resolve(self.settings.theme, self.system_mode)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let theme = if self.settings.theme == ThemePref::System {
            system::theme_changes().map(Message::SystemTheme)
        } else {
            Subscription::none()
        };
        let drops = event::listen_with(|event, _, _| match event {
            Event::Window(window::Event::FileDropped(path)) => Some(Message::FileDropped(path)),
            _ => None,
        });
        let bridge = if self.dev.demo {
            Subscription::none()
        } else {
            browser::subscription()
        };
        Subscription::batch([theme, drops, bridge, self.frames()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App::new(Settings::default(), None, DevEnv::default())
    }

    #[test]
    fn system_pref_follows_os_mode() {
        let mut app = app();
        let _ = app.update(Message::SystemTheme(Mode::Dark));
        assert!(theme::is_dark(&app.theme()));
        let _ = app.update(Message::SystemTheme(Mode::Light));
        assert!(!theme::is_dark(&app.theme()));
    }

    #[test]
    fn explicit_pref_overrides_os_mode() {
        let mut app = app();
        let _ = app.update(Message::SystemTheme(Mode::Dark));
        let _ = app.update(Message::SetThemePref(ThemePref::Light));
        assert!(!theme::is_dark(&app.theme()));
        let _ = app.update(Message::SetThemePref(ThemePref::Dark));
        let _ = app.update(Message::SystemTheme(Mode::Light));
        assert!(theme::is_dark(&app.theme()));
    }

    #[tokio::test]
    async fn set_lang_updates_settings_and_saves() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("t.db")).await.unwrap());
        let mut app = App::new(Settings::default(), Some(store), DevEnv::default());

        let task = app.update(Message::SetLang(Lang::En));
        assert_eq!(app.settings.lang, Lang::En);
        assert_eq!(task.units(), 1);
    }

    #[test]
    fn set_lang_without_store_saves_nothing() {
        let mut app = app();
        let task = app.update(Message::SetLang(Lang::En));
        assert_eq!(app.settings.lang, Lang::En);
        assert_eq!(task.units(), 0);
    }

    #[test]
    fn navigate_switches_screen() {
        let mut app = app();
        let _ = app.update(Message::Navigate(Screen::Settings));
        assert_eq!(app.screen, Screen::Settings);
    }

    #[test]
    fn dev_env_overrides_loaded_settings() {
        let dev = DevEnv {
            theme: Some(ThemePref::Dark),
            lang: Some(Lang::En),
            demo: true,
            ..DevEnv::default()
        };
        let mut app = App::new(Settings::default(), None, dev);
        assert!(app.result.is_some());
        let _ = app.update(Message::Booted(None, Box::default()));
        assert_eq!(app.settings.theme, ThemePref::Dark);
        assert_eq!(app.settings.lang, Lang::En);
    }
}
