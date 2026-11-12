mod header;
pub mod snap;

use std::fmt;
use std::sync::Arc;

use iced::theme::Mode;
use iced::widget::{column, container, text};
use iced::{Element, Length, Subscription, Task, Theme, system, window};

use crate::demo::demo_comparison;
use crate::i18n::{Key, Lang, t};
use crate::pipeline::{CompareInput, ComparisonResult};
use crate::settings::{Settings, ThemePref};
use crate::store::{Store, default_db_path};
use crate::theme::{self, tokens};
use snap::DevEnv;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Compare,
    History,
    Settings,
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
    TakeSnapshot,
    Snapshot(window::Screenshot),
}

pub struct App {
    pub settings: Settings,
    pub system_mode: Mode,
    pub screen: Screen,
    pub input: Option<CompareInput>,
    pub result: Option<ComparisonResult>,
    store: Option<Arc<Store>>,
    dev: DevEnv,
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
        Self {
            settings,
            system_mode: Mode::None,
            screen: dev.screen.unwrap_or_default(),
            input,
            result,
            store,
            dev,
        }
    }

    pub fn boot() -> (Self, Task<Message>) {
        let dev = DevEnv::from_env();
        let mut tasks = vec![
            system::theme().map(Message::SystemTheme),
            Task::perform(open_store(), |(store, settings)| {
                Message::Booted(store, settings)
            }),
        ];
        if dev.snapshot.is_some() {
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
                Task::none()
            }
            Message::Booted(store, mut settings) => {
                self.dev.apply(&mut settings);
                self.settings = *settings;
                self.store = store.map(|h| h.0);
                Task::none()
            }
            Message::Saved(result) => {
                if let Err(e) = result {
                    eprintln!("maeve: cannot save settings: {e}");
                }
                Task::none()
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
        let t_ = tokens(theme::is_dark(&self.theme()));

        let title = match self.screen {
            Screen::Compare => t(lang, Key::Compare),
            Screen::History => t(lang, Key::NavHistory),
            Screen::Settings => t(lang, Key::NavSettings),
        };
        let mut body = column![text(title).size(22)].spacing(12);
        if let Some(result) = &self.result
            && self.screen == Screen::Compare
        {
            body = body
                .push(
                    text(&result.products[result.judge.winner].title)
                        .size(18)
                        .color(t_.primary),
                )
                .push(text(&result.judge.verdict).style(theme::text_muted));
        }

        column![
            header::view(lang, self.settings.theme, self.screen),
            container(
                container(body)
                    .padding(24)
                    .width(Length::Fill)
                    .style(theme::card)
            )
            .padding(24)
            .width(Length::Fill)
            .height(Length::Fill),
        ]
        .into()
    }

    pub fn theme(&self) -> Theme {
        theme::resolve(self.settings.theme, self.system_mode)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if self.settings.theme == ThemePref::System {
            system::theme_changes().map(Message::SystemTheme)
        } else {
            Subscription::none()
        }
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
