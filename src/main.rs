use iced::{Size, window};
use maeve::i18n::{Key, Lang, t};
use maeve::ui::App;

fn main() -> iced::Result {
    iced::application(App::boot, App::update, App::view)
        .title(t(Lang::Es, Key::AppName))
        .theme(App::theme)
        .subscription(App::subscription)
        .window(window::Settings {
            size: Size::new(1100.0, 800.0),
            icon: window::icon::from_file_data(include_bytes!("../assets/icons/256x256.png"), None)
                .ok(),
            ..window::Settings::default()
        })
        .run()
}
