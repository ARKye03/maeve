mod styles;

use std::sync::LazyLock;

use iced::theme::{Mode, Palette};
use iced::{Color, Theme};

pub use crate::settings::ThemePref;
pub use styles::{
    bar, button_destructive, button_ghost, button_primary, button_secondary, card, chip, header,
    input, segment, segmented, table_row, text_muted,
};

pub const RADIUS: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    pub background: Color,
    pub foreground: Color,
    pub card: Color,
    pub card_foreground: Color,
    pub primary: Color,
    pub primary_foreground: Color,
    pub secondary: Color,
    pub secondary_foreground: Color,
    pub muted: Color,
    pub muted_foreground: Color,
    pub accent: Color,
    pub accent_foreground: Color,
    pub destructive: Color,
    pub destructive_foreground: Color,
    pub border: Color,
    pub input: Color,
    pub ring: Color,
    pub chart: [Color; 5],
}

/// OKLCH (hue in degrees) to gamut-clipped sRGB, via Ottosson's OKLab matrices.
pub fn oklch(l: f32, c: f32, h: f32) -> Color {
    let (l, c, h) = (f64::from(l), f64::from(c), f64::from(h).to_radians());
    let (a, b) = (c * h.cos(), c * h.sin());

    let l_ = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m_ = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s_ = (l - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);

    let r = 4.076_741_662_1 * l_ - 3.307_711_591_3 * m_ + 0.230_969_929_2 * s_;
    let g = -1.268_438_004_6 * l_ + 2.609_757_401_1 * m_ - 0.341_319_396_5 * s_;
    let bl = -0.004_196_086_3 * l_ - 0.703_418_614_7 * m_ + 1.707_614_701_0 * s_;

    let gamma = |x: f64| {
        let x = x.clamp(0.0, 1.0);
        let v = if x <= 0.003_130_8 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        };
        v.clamp(0.0, 1.0) as f32
    };
    Color::from_rgb(gamma(r), gamma(g), gamma(bl))
}

fn gray(l: f32) -> Color {
    oklch(l, 0.0, 0.0)
}

fn light_tokens() -> Tokens {
    let primary = oklch(0.4341, 0.0392, 41.9938);
    Tokens {
        background: gray(0.9821),
        foreground: gray(0.2435),
        card: gray(0.9911),
        card_foreground: gray(0.2435),
        primary,
        primary_foreground: gray(1.0),
        secondary: oklch(0.92, 0.0651, 74.3695),
        secondary_foreground: oklch(0.3499, 0.0685, 40.8288),
        muted: gray(0.9521),
        muted_foreground: gray(0.5032),
        accent: gray(0.931),
        accent_foreground: gray(0.2435),
        destructive: oklch(0.6271, 0.1936, 33.339),
        destructive_foreground: gray(1.0),
        border: gray(0.8822),
        input: gray(0.8822),
        ring: primary,
        chart: [
            oklch(0.4341, 0.0392, 41.99),
            oklch(0.92, 0.0651, 74.37),
            gray(0.931),
            oklch(0.9367, 0.0523, 75.50),
            oklch(0.4338, 0.0437, 41.67),
        ],
    }
}

fn dark_tokens() -> Tokens {
    let primary = oklch(0.9247, 0.0524, 66.1732);
    Tokens {
        background: gray(0.1776),
        foreground: gray(0.9491),
        card: gray(0.2134),
        card_foreground: gray(0.9491),
        primary,
        primary_foreground: oklch(0.2029, 0.024, 200.1962),
        secondary: oklch(0.3163, 0.019, 63.6992),
        secondary_foreground: oklch(0.9247, 0.0524, 66.1732),
        muted: gray(0.252),
        muted_foreground: gray(0.7699),
        accent: gray(0.285),
        accent_foreground: gray(0.9491),
        destructive: oklch(0.6271, 0.1936, 33.339),
        destructive_foreground: gray(1.0),
        border: oklch(0.2351, 0.0115, 91.7467),
        input: gray(0.4017),
        ring: primary,
        chart: [
            oklch(0.9247, 0.0524, 66.17),
            oklch(0.3163, 0.019, 63.70),
            gray(0.285),
            oklch(0.3481, 0.0219, 67.00),
            oklch(0.9245, 0.0533, 67.09),
        ],
    }
}

static LIGHT: LazyLock<Tokens> = LazyLock::new(light_tokens);
static DARK: LazyLock<Tokens> = LazyLock::new(dark_tokens);

pub fn tokens(dark: bool) -> &'static Tokens {
    if dark { &DARK } else { &LIGHT }
}

fn theme_for(dark: bool) -> Theme {
    let t = tokens(dark);
    let palette = Palette {
        background: t.background,
        text: t.foreground,
        primary: t.primary,
        success: t.chart[0],
        warning: t.secondary,
        danger: t.destructive,
    };
    Theme::custom(if dark { "Maeve Dark" } else { "Maeve Light" }, palette)
}

/// `Mode::None` (OS gave no preference) falls back to light.
pub fn resolve(pref: ThemePref, system: Mode) -> Theme {
    let dark = match pref {
        ThemePref::Dark => true,
        ThemePref::Light => false,
        ThemePref::System => system == Mode::Dark,
    };
    theme_for(dark)
}

pub fn is_dark(theme: &Theme) -> bool {
    let bg = theme.palette().background;
    0.2126 * bg.r + 0.7152 * bg.g + 0.0722 * bg.b < 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(c: Color, hex: u32, tol: f32) -> bool {
        let want = [(hex >> 16) & 0xff, (hex >> 8) & 0xff, hex & 0xff];
        let got = [c.r, c.g, c.b];
        want.iter()
            .zip(got)
            .all(|(w, g)| (g * 255.0 - *w as f32).abs() <= tol)
    }

    #[test]
    fn oklch_white() {
        let c = oklch(1.0, 0.0, 0.0);
        for ch in [c.r, c.g, c.b] {
            assert!((ch - 1.0).abs() <= 0.002, "{c:?}");
        }
    }

    #[test]
    fn oklch_black() {
        assert_eq!(oklch(0.0, 0.0, 0.0), Color::BLACK);
    }

    #[test]
    fn oklch_light_primary() {
        let c = oklch(0.4341, 0.0392, 41.9938);
        assert!(near(c, 0x644a40, 2.0), "{c:?}");
    }

    #[test]
    fn oklch_dark_primary() {
        let c = oklch(0.9247, 0.0524, 66.1732);
        assert!(near(c, 0xffe0c2, 2.0), "{c:?}");
    }

    #[test]
    fn oklch_out_of_gamut_is_clipped() {
        let c = oklch(0.6271, 0.4, 33.339);
        for ch in [c.r, c.g, c.b] {
            assert!((0.0..=1.0).contains(&ch));
        }
    }

    #[test]
    fn resolve_honours_pref() {
        assert!(is_dark(&resolve(ThemePref::System, Mode::Dark)));
        assert!(!is_dark(&resolve(ThemePref::System, Mode::Light)));
        assert!(!is_dark(&resolve(ThemePref::System, Mode::None)));
        assert!(!is_dark(&resolve(ThemePref::Light, Mode::Dark)));
        assert!(is_dark(&resolve(ThemePref::Dark, Mode::Light)));
    }

    #[test]
    fn palette_mapping() {
        let t = tokens(false);
        let p = resolve(ThemePref::Light, Mode::None).palette();
        assert_eq!(p.background, t.background);
        assert_eq!(p.text, t.foreground);
        assert_eq!(p.primary, t.primary);
        assert_eq!(p.success, t.chart[0]);
        assert_eq!(p.warning, t.secondary);
        assert_eq!(p.danger, t.destructive);
    }

    #[test]
    fn theme_names() {
        assert_eq!(
            resolve(ThemePref::Light, Mode::None).to_string(),
            "Maeve Light"
        );
        assert_eq!(
            resolve(ThemePref::Dark, Mode::None).to_string(),
            "Maeve Dark"
        );
    }
}
