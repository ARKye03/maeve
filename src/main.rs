use iced::Size;
use maeve::i18n::{Key, Lang, t};
use maeve::ui::App;

fn main() -> iced::Result {
    iced::application(App::boot, App::update, App::view)
        .title(t(Lang::Es, Key::AppName))
        .theme(App::theme)
        .subscription(App::subscription)
        .window_size(Size::new(1100.0, 800.0))
        .run()
}
