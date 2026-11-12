//! Loading state while a comparison runs: spinner, stage steps, rotating
//! phrases and a pulsing skeleton of the results card.

use std::f32::consts::{PI, TAU};
use std::time::Duration;

use iced::widget::{Space, canvas, column, container, row, text};
use iced::{
    Alignment, Background, Border, Color, Element, Length, Point, Rectangle, Renderer, Theme, mouse,
};

use super::results::divider;
use super::{App, ItemStatus, Message, RunState};
use crate::i18n::{Key, Lang, t};
use crate::theme::{self, Tokens, tokens};

const FRAME: Duration = Duration::from_millis(33);
const PHRASE_EVERY: Duration = Duration::from_secs(6);
const PULSE_PERIOD_S: f32 = 1.4;
const SPIN_PERIOD_S: f32 = 1.0;

const PHRASES: &[(&str, &str)] = &[
    (
        "Leyendo la letra chica para que tú no tengas que hacerlo.",
        "Reading the fine print so you don't have to.",
    ),
    (
        "Sumando envío, IVA y tipo de cambio…",
        "Adding up shipping, VAT and the exchange rate…",
    ),
    (
        "Cada producto tiene su defensor; el juez escucha a todos.",
        "Every product gets an advocate; the judge hears them all.",
    ),
    (
        "Comparando reseñas, no solo estrellas.",
        "Weighing reviews, not just stars.",
    ),
    (
        "Lo barato puede salir caro. Lo estamos verificando.",
        "Cheap can turn out expensive. Checking that.",
    ),
    (
        "Calculando el costo real puesto en Uruguay.",
        "Working out the real landed cost in Uruguay.",
    ),
    (
        "Un buen precio no compensa una mala garantía.",
        "A good price doesn't make up for a bad warranty.",
    ),
    (
        "Los modelos están deliberando. Ningún producto fue sobornado.",
        "The models are deliberating. No product was bribed.",
    ),
    (
        "Revisando las especificaciones que nadie lee.",
        "Checking the specs nobody reads.",
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Done,
    Current,
    Pending,
}

impl App {
    /// Needs animation frames: a run, or a row being read or extracted.
    pub(super) fn animating(&self) -> bool {
        self.running()
            || self
                .items
                .iter()
                .any(|i| matches!(i.status, ItemStatus::Scraping | ItemStatus::Extracting))
    }

    pub(super) fn frames(&self) -> iced::Subscription<Message> {
        if self.animating() && self.dev.snapshot.is_none() {
            iced::time::every(FRAME).map(Message::Tick)
        } else {
            iced::Subscription::none()
        }
    }

    /// Screenshot fixture: second row still being read, mid-pulse.
    pub(super) fn show_loading(&mut self) {
        self.run_items = (0..self.items.len()).collect();
        if let Some(item) = self.items.get_mut(1) {
            item.status = ItemStatus::Scraping;
        }
        self.run = RunState::Running {
            advocates_done: 0,
            waiting: None,
            judging: false,
        };
        self.now = self.anim_start + Duration::from_millis(500);
    }

    fn elapsed(&self) -> f32 {
        self.now
            .saturating_duration_since(self.anim_start)
            .as_secs_f32()
    }

    /// 0 → 1 → 0 over one pulse period.
    pub(super) fn pulse(&self) -> f32 {
        0.5 - 0.5 * (self.elapsed() * TAU / PULSE_PERIOD_S).cos()
    }

    fn phrase(&self, lang: Lang) -> &'static str {
        let n = (self.elapsed() / PHRASE_EVERY.as_secs_f32()) as usize + self.phrase_seed;
        let (es, en) = PHRASES[n % PHRASES.len()];
        match lang {
            Lang::Es => es,
            Lang::En => en,
        }
    }

    fn steps(&self) -> [(Key, Step, Option<String>); 3] {
        let (advocates_done, judging) = match &self.run {
            RunState::Running {
                advocates_done,
                judging,
                ..
            } => (*advocates_done, *judging),
            _ => (0, false),
        };
        let scraping = self
            .run_items
            .iter()
            .any(|&i| matches!(self.items[i].status, ItemStatus::Scraping));
        let ready = self
            .run_items
            .iter()
            .filter(|&&i| matches!(self.items[i].status, ItemStatus::Ready(_)))
            .count();
        let (read, advocates, judge) = if scraping {
            (Step::Current, Step::Pending, Step::Pending)
        } else if judging {
            (Step::Done, Step::Done, Step::Current)
        } else {
            (Step::Done, Step::Current, Step::Pending)
        };
        let count = (!scraping).then(|| format!("{advocates_done}/{ready}"));
        [
            (Key::StageRead, read, None),
            (Key::ProgressAdvocates, advocates, count),
            (Key::StageVerdict, judge, None),
        ]
    }

    pub(super) fn loading_card(&self) -> Option<Element<'_, Message>> {
        if !self.running() {
            return None;
        }
        let lang = self.settings.lang;
        let t_ = tokens(theme::is_dark(&self.theme()));
        let pulse = self.pulse();

        let spinner = canvas(Spinner {
            angle: self.elapsed() * TAU / SPIN_PERIOD_S,
            arc: t_.primary,
            track: t_.border,
        })
        .width(28)
        .height(28);
        let headline = column![
            text(self.progress_text().unwrap_or_default()).size(16),
            text(self.phrase(lang)).size(13).style(theme::text_muted),
        ]
        .spacing(4);

        let mut steps = row![].spacing(20).align_y(Alignment::Center);
        for (key, step, extra) in self.steps() {
            let label = match extra {
                Some(extra) => format!("{} {extra}", t(lang, key)),
                None => t(lang, key).to_owned(),
            };
            let dot_color = match step {
                Step::Done => t_.primary,
                Step::Current => blend(t_.muted_foreground, t_.primary, 0.35 + 0.65 * pulse),
                Step::Pending => blend(t_.card, t_.muted_foreground, 0.45),
            };
            let label = text(label).size(13);
            let label = if step == Step::Pending {
                label.style(theme::text_muted)
            } else {
                label
            };
            steps = steps.push(
                row![dot(dot_color), label]
                    .spacing(8)
                    .align_y(Alignment::Center),
            );
        }

        let card = column![
            row![spinner, headline]
                .spacing(16)
                .align_y(Alignment::Center),
            steps,
            divider(),
            skeleton_results(t_, pulse),
        ]
        .spacing(20);
        Some(
            container(card)
                .padding(24)
                .width(Length::Fill)
                .style(theme::card)
                .into(),
        )
    }
}

/// Placeholder bar for content that is still loading.
pub(super) fn bar<'a>(
    t_: &'static Tokens,
    pulse: f32,
    width: Length,
    height: f32,
) -> Element<'a, Message> {
    let color = blend(
        t_.muted,
        blend(t_.border, t_.muted_foreground, 0.25),
        0.2 + 0.8 * pulse,
    );
    container(Space::new())
        .width(width)
        .height(height)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                radius: (height / 2.0).min(6.0).into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

fn skeleton_results<'a>(t_: &'static Tokens, pulse: f32) -> Element<'a, Message> {
    let line = |w: Length, h: f32| bar(t_, pulse, w, h);
    let mut table = column![].spacing(14);
    for _ in 0..4 {
        table = table.push(
            row![
                line(Length::FillPortion(2), 12.0),
                line(Length::FillPortion(3), 12.0),
                line(Length::FillPortion(3), 12.0),
            ]
            .spacing(32),
        );
    }
    column![
        line(Length::Fixed(90.0), 10.0),
        row![
            line(Length::FillPortion(3), 26.0),
            Space::new().width(Length::FillPortion(2))
        ],
        column![
            line(Length::Fill, 12.0),
            row![
                line(Length::FillPortion(4), 12.0),
                Space::new().width(Length::FillPortion(1))
            ],
        ]
        .spacing(8),
        divider(),
        table,
    ]
    .spacing(14)
    .into()
}

fn dot<'a>(color: Color) -> Element<'a, Message> {
    container(Space::new())
        .width(8)
        .height(8)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                radius: 4.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

fn blend(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

struct Spinner {
    angle: f32,
    arc: Color,
    track: Color,
}

impl canvas::Program<Message> for Spinner {
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
        let width = 3.0;
        let center = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let radius = bounds.width.min(bounds.height) / 2.0 - width;
        let stroke = |color| {
            canvas::Stroke::default()
                .with_color(color)
                .with_width(width)
                .with_line_cap(canvas::LineCap::Round)
        };
        frame.stroke(&canvas::Path::circle(center, radius), stroke(self.track));
        let arc = canvas::Path::new(|p| {
            p.arc(canvas::path::Arc {
                center,
                radius,
                start_angle: self.angle.into(),
                end_angle: (self.angle + PI * 1.4).into(),
            });
        });
        frame.stroke(&arc, stroke(self.arc));
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_phrase_has_both_languages() {
        for (es, en) in PHRASES {
            assert!(!es.is_empty() && !en.is_empty());
        }
    }

    #[test]
    fn pulse_stays_in_range() {
        let mut app = App::new(
            crate::settings::Settings::default(),
            None,
            super::super::snap::DevEnv::default(),
        );
        for ms in (0..3000).step_by(70) {
            app.now = app.anim_start + Duration::from_millis(ms);
            let p = app.pulse();
            assert!((0.0..=1.0).contains(&p), "{p}");
        }
    }

    #[test]
    fn loading_card_only_while_running() {
        let mut app = App::new(
            crate::settings::Settings::default(),
            None,
            super::super::snap::DevEnv::default(),
        );
        assert!(app.loading_card().is_none());
        app.run = RunState::Running {
            advocates_done: 0,
            waiting: None,
            judging: false,
        };
        assert!(app.loading_card().is_some());
    }
}
