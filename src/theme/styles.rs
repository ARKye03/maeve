use iced::widget::{button, container, progress_bar, text, text_input};
use iced::{Background, Border, Color, Shadow, Theme, Vector};

use super::{RADIUS, Tokens, is_dark, tokens};

fn tk(theme: &Theme) -> &'static Tokens {
    tokens(is_dark(theme))
}

fn shadow() -> Shadow {
    Shadow {
        color: Color::from_rgba(0.0, 0.0, 0.0, 0.1),
        offset: Vector::new(0.0, 1.0),
        blur_radius: 3.0,
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

fn rounded(width: f32, color: Color) -> Border {
    Border {
        color,
        width,
        radius: RADIUS.into(),
    }
}

fn filled(
    bg: Color,
    fg: Color,
    border: Border,
    status: button::Status,
    hover: Color,
) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let style = button::Style {
        background: Some(Background::Color(if hovered { hover } else { bg })),
        text_color: fg,
        border,
        shadow: Shadow::default(),
        snap: true,
    };
    match status {
        button::Status::Disabled => button::Style {
            background: style.background.map(|b| b.scale_alpha(0.5)),
            text_color: Color { a: 0.5, ..fg },
            border: Border {
                color: Color {
                    a: border.color.a * 0.5,
                    ..border.color
                },
                ..border
            },
            ..style
        },
        _ => style,
    }
}

pub fn button_primary(theme: &Theme, status: button::Status) -> button::Style {
    let t = tk(theme);
    let mut s = filled(
        t.primary,
        t.primary_foreground,
        rounded(0.0, Color::TRANSPARENT),
        status,
        mix(t.primary, t.foreground, 0.12),
    );
    s.shadow = shadow();
    s
}

pub fn button_secondary(theme: &Theme, status: button::Status) -> button::Style {
    let t = tk(theme);
    filled(
        t.secondary,
        t.secondary_foreground,
        rounded(0.0, Color::TRANSPARENT),
        status,
        mix(t.secondary, t.secondary_foreground, 0.1),
    )
}

pub fn button_destructive(theme: &Theme, status: button::Status) -> button::Style {
    let t = tk(theme);
    filled(
        t.destructive,
        t.destructive_foreground,
        rounded(0.0, Color::TRANSPARENT),
        status,
        mix(t.destructive, t.foreground, 0.12),
    )
}

pub fn button_ghost(theme: &Theme, status: button::Status) -> button::Style {
    let t = tk(theme);
    filled(
        Color::TRANSPARENT,
        t.foreground,
        rounded(0.0, Color::TRANSPARENT),
        status,
        t.accent,
    )
}

pub fn chip(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tk(theme);
        if selected {
            filled(
                t.primary,
                t.primary_foreground,
                rounded(1.0, t.primary),
                status,
                mix(t.primary, t.foreground, 0.12),
            )
        } else {
            filled(
                t.card,
                t.foreground,
                rounded(1.0, t.border),
                status,
                t.accent,
            )
        }
    }
}

pub fn card(theme: &Theme) -> container::Style {
    let t = tk(theme);
    container::Style {
        text_color: Some(t.card_foreground),
        background: Some(Background::Color(t.card)),
        border: rounded(1.0, t.border),
        shadow: shadow(),
        snap: true,
    }
}

pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let t = tk(theme);
    let border_color = match status {
        text_input::Status::Focused { .. } => t.ring,
        text_input::Status::Hovered => mix(t.input, t.foreground, 0.25),
        _ => t.input,
    };
    let style = text_input::Style {
        background: Background::Color(t.background),
        border: rounded(1.0, border_color),
        icon: t.muted_foreground,
        placeholder: t.muted_foreground,
        value: t.foreground,
        selection: Color { a: 0.3, ..t.ring },
    };
    match status {
        text_input::Status::Disabled => text_input::Style {
            background: Background::Color(t.muted),
            value: t.muted_foreground,
            ..style
        },
        _ => style,
    }
}

pub fn text_muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(tk(theme).muted_foreground),
    }
}

pub fn table_row(theme: &Theme) -> container::Style {
    let t = tk(theme);
    container::Style {
        text_color: Some(t.foreground),
        border: Border {
            color: t.border,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn list_item(theme: &Theme) -> container::Style {
    let t = tk(theme);
    container::Style {
        text_color: Some(t.foreground),
        border: rounded(1.0, t.border),
        ..container::Style::default()
    }
}

pub fn bar(series: usize) -> impl Fn(&Theme) -> progress_bar::Style {
    move |theme| {
        let t = tk(theme);
        progress_bar::Style {
            background: Background::Color(t.muted),
            bar: Background::Color(t.chart[series % t.chart.len()]),
            border: Border {
                radius: (RADIUS / 2.0).into(),
                ..Border::default()
            },
        }
    }
}

pub fn header(theme: &Theme) -> container::Style {
    let t = tk(theme);
    container::Style {
        text_color: Some(t.card_foreground),
        background: Some(Background::Color(t.card)),
        border: Border {
            color: t.border,
            width: 1.0,
            radius: 0.0.into(),
        },
        shadow: shadow(),
        snap: true,
    }
}

pub fn segmented(theme: &Theme) -> container::Style {
    let t = tk(theme);
    container::Style {
        background: Some(Background::Color(t.muted)),
        border: rounded(0.0, Color::TRANSPARENT),
        ..container::Style::default()
    }
}

pub fn segment(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tk(theme);
        if selected {
            let mut s = filled(
                t.card,
                t.foreground,
                rounded(0.0, Color::TRANSPARENT),
                status,
                t.card,
            );
            s.shadow = shadow();
            s
        } else {
            filled(
                Color::TRANSPARENT,
                t.muted_foreground,
                rounded(0.0, Color::TRANSPARENT),
                status,
                t.accent,
            )
        }
    }
}
