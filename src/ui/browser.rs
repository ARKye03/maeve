//! Pages captured by the browser extension (see `crate::bridge`) fill in rows
//! whose scrape failed, e.g. when TEMU shows the scraper a captcha.

use std::sync::Arc;

use iced::futures::SinkExt;
use iced::{Subscription, Task};
use url::Url;

use super::{App, ItemStatus, Message};
use crate::bridge::{self, Page};
use crate::domain::Site;
use crate::scrape::classify_url;

/// Identity of a product page, so the page the browser reports (with tracking
/// parameters, a different slug, …) matches the URL the user pasted.
pub(super) fn product_key(url: &str) -> Option<(Site, String)> {
    let (site, url) = classify_url(url).ok()?;
    let id = match site {
        Site::Temu => temu_goods_id(&url)?,
        Site::MercadoLibre => mercadolibre_item_id(&url)?,
    };
    Some((site, id))
}

fn temu_goods_id(url: &Url) -> Option<String> {
    if let Some((_, id)) = url.query_pairs().find(|(k, _)| k == "goods_id") {
        return Some(id.into_owned());
    }
    let last = url.path_segments()?.next_back()?;
    let stem = last.strip_suffix(".html")?;
    let (_, id) = stem.rsplit_once("-g-")?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_owned())
}

fn mercadolibre_item_id(url: &Url) -> Option<String> {
    let path = url.path().to_ascii_uppercase();
    let start = path.find("MLU")?;
    let id: String = path[start + 3..]
        .trim_start_matches('-')
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    (!id.is_empty()).then(|| format!("MLU{id}"))
}

pub(super) fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let program = "xdg-open";
    if let Err(e) = std::process::Command::new(program).arg(url).spawn() {
        eprintln!("maeve: cannot open the browser: {e}");
    }
}

pub(super) fn subscription() -> Subscription<Message> {
    Subscription::run(|| {
        iced::stream::channel(4, async |mut output| {
            let listener = match bridge::listen().await {
                Ok(l) => l,
                Err(e) => {
                    eprintln!(
                        "maeve: browser extension bridge disabled (port {}): {e}",
                        bridge::PORT
                    );
                    return;
                }
            };
            let (tx, mut rx) = tokio::sync::mpsc::channel(4);
            tokio::spawn(async move {
                if let Err(e) = bridge::serve(listener, tx).await {
                    eprintln!("maeve: browser extension bridge stopped: {e}");
                }
            });
            while let Some(page) = rx.recv().await {
                if output
                    .send(Message::BrowserPage(Box::new(page)))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        })
    })
}

impl App {
    pub(super) fn open_in_browser(&self, i: usize) {
        if let Some(item) = self.items.get(i)
            && let Ok((_, url)) = classify_url(&item.url)
        {
            open_url(url.as_str());
        }
    }

    pub(super) fn browser_page(&mut self, page: Page) -> Task<Message> {
        if self.running() {
            return Task::none();
        }
        let Some(key) = product_key(&page.url) else {
            return Task::none();
        };
        let Some(i) = self.items.iter().position(|item| {
            matches!(item.status, ItemStatus::Failed(_))
                && product_key(&item.url).as_ref() == Some(&key)
        }) else {
            return Task::none();
        };
        let Ok((_, url)) = classify_url(&page.url) else {
            return Task::none();
        };
        let item = &mut self.items[i];
        item.status = ItemStatus::Extracting;
        item.paste_error = None;
        let scraper = Arc::clone(&self.scraper);
        Task::perform(
            async move {
                scraper
                    .parse_page(url, page.html)
                    .await
                    .map(Box::new)
                    .map_err(|e| e.message)
            },
            move |result| Message::Extracted(i, result),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrape::{ScrapeError, ScrapeErrorKind};
    use crate::settings::Settings;
    use crate::ui::snap::DevEnv;

    const PASTED: &str = "https://www.temu.com/uy/cesta-de-filtro-g-601100900000000.html";

    #[test]
    fn temu_pages_match_by_goods_id() {
        let browser =
            "https://www.temu.com/uy/otra-cosa-g-601100900000000.html?_x_sessn_id=1&refer=2";
        assert_eq!(product_key(PASTED), product_key(browser));
        assert_eq!(
            product_key("https://www.temu.com/goods.html?goods_id=601100900000000"),
            product_key(PASTED)
        );
        assert_ne!(
            product_key("https://www.temu.com/uy/cesta-g-1.html"),
            product_key(PASTED)
        );
        assert_eq!(product_key("https://www.temu.com/uy/"), None);
    }

    #[test]
    fn mercadolibre_pages_match_by_item_id() {
        assert_eq!(
            product_key("https://articulo.mercadolibre.com.uy/MLU-123456-mini-pc-_JM"),
            Some((Site::MercadoLibre, "MLU123456".into()))
        );
    }

    fn failed_app() -> App {
        let mut app = App::new(Settings::default(), None, DevEnv::default());
        app.items[0].url = PASTED.into();
        app.items[0].status =
            ItemStatus::Failed(ScrapeError::new(ScrapeErrorKind::Blocked, "captcha"));
        app
    }

    fn page(url: &str) -> Page {
        Page {
            url: url.into(),
            html: "<html></html>".into(),
        }
    }

    #[test]
    fn a_matching_page_starts_parsing_the_failed_row() {
        let mut app = failed_app();
        let task = app.browser_page(page(
            "https://www.temu.com/uy/x-g-601100900000000.html?refer=1",
        ));
        assert_eq!(task.units(), 1);
        assert!(matches!(app.items[0].status, ItemStatus::Extracting));
    }

    #[test]
    fn unrelated_pages_and_healthy_rows_are_ignored() {
        let mut app = failed_app();
        assert_eq!(
            app.browser_page(page("https://www.temu.com/uy/x-g-1.html"))
                .units(),
            0
        );
        app.items[0].status = ItemStatus::Idle;
        assert_eq!(app.browser_page(page(PASTED)).units(), 0);
    }
}
