use iced::widget::{Space, button, container, row, text};
use iced::{Alignment, Element, Length};

use super::{Message, Screen};
use crate::i18n::{Key, Lang, t};
use crate::theme::{self, ThemePref};

pub(super) fn segment<'a>(
    label: String,
    selected: bool,
    on_press: Message,
) -> iced::widget::Button<'a, Message> {
    let size = if label.chars().count() == 1 { 17 } else { 13 };
    button(
        text(label)
            .size(size)
            .shaping(text::Shaping::Advanced)
            .center(),
    )
    .height(28)
    .padding([0, 12])
    .style(theme::segment(selected))
    .on_press(on_press)
}

pub(super) fn segmented<'a>(
    items: impl IntoIterator<Item = iced::widget::Button<'a, Message>>,
) -> Element<'a, Message> {
    container(row(items.into_iter().map(Element::from)).spacing(4))
        .padding(4)
        .style(theme::segmented)
        .into()
}

fn nav<'a>(label: &'static str, active: bool, to: Screen) -> Element<'a, Message> {
    button(text(label).size(14))
        .padding([8, 12])
        .style(if active {
            theme::button_secondary
        } else {
            theme::button_ghost
        })
        .on_press(Message::Navigate(to))
        .into()
}

pub fn view(lang: Lang, pref: ThemePref, screen: Screen) -> Element<'static, Message> {
    let theme_toggle = segmented([
        segment(
            "\u{2600}".into(),
            pref == ThemePref::Light,
            Message::SetThemePref(ThemePref::Light),
        ),
        segment(
            "\u{263E}".into(),
            pref == ThemePref::Dark,
            Message::SetThemePref(ThemePref::Dark),
        ),
        segment(
            t(lang, Key::ThemeSystem).into(),
            pref == ThemePref::System,
            Message::SetThemePref(ThemePref::System),
        ),
    ]);

    let lang_toggle = segmented([
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

    let name = button(text(t(lang, Key::AppName)).size(20))
        .padding([4, 8])
        .style(theme::button_ghost)
        .on_press(Message::Navigate(Screen::Compare));

    container(
        row![
            name,
            Space::new().width(Length::Fill),
            theme_toggle,
            lang_toggle,
            nav(
                t(lang, Key::NavHistory),
                screen == Screen::History,
                Screen::History
            ),
            nav(
                t(lang, Key::NavSettings),
                screen == Screen::Settings,
                Screen::Settings
            ),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding([12, 20])
    .width(Length::Fill)
    .style(theme::header)
    .into()
}
