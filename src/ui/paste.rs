//! Paste-instead fallback: extract a product from pasted text or a screenshot
//! when scraping fails.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::{Task, clipboard};

use super::{App, ItemStatus, Message};
use crate::domain::{Product, Site};
use crate::i18n::{Key, t};
use crate::llm::prompts::extraction_request;
use crate::llm::schema::{extraction_into_product, parse_extraction};
use crate::llm::{Llm, Part};
use crate::scrape::{ScrapeError, ScrapeErrorKind, classify_url};

const MAX_TEXT_CHARS: usize = 60_000;
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// MIME type and encoded bytes.
pub type PastedImage = (String, Vec<u8>);

pub(super) async fn extract<L: Llm>(
    llm: Arc<L>,
    site: Site,
    url: String,
    parts: Vec<Part>,
    model: String,
) -> Result<Box<Product>, String> {
    let request = extraction_request(site, parts, &model);
    let value = llm
        .generate_json(request)
        .await
        .map_err(|e| e.to_string())?;
    let extraction = parse_extraction(value).map_err(|e| e.to_string())?;
    Ok(Box::new(extraction_into_product(extraction, site, url)))
}

fn encode_png(width: usize, height: usize, rgba: &[u8]) -> Option<Vec<u8>> {
    if rgba.len() != width.checked_mul(height)?.checked_mul(4)? {
        return None;
    }
    let mut out = Cursor::new(Vec::new());
    image::write_buffer_with_format(
        &mut out,
        rgba,
        u32::try_from(width).ok()?,
        u32::try_from(height).ok()?,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .ok()?;
    Some(out.into_inner())
}

fn read_clipboard_image() -> Result<PastedImage, Key> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| {
        eprintln!("maeve: cannot open clipboard: {e}");
        Key::ClipboardNoImage
    })?;
    let img = clipboard.get_image().map_err(|_| Key::ClipboardNoImage)?;
    let png = encode_png(img.width, img.height, &img.bytes).ok_or(Key::ImageUnreadable)?;
    if png.len() > MAX_IMAGE_BYTES {
        return Err(Key::ImageUnreadable);
    }
    Ok(("image/png".into(), png))
}

fn image_mime(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

fn load_image_file(path: &Path) -> Result<PastedImage, Key> {
    let mime = image_mime(path).ok_or(Key::ImageUnreadable)?;
    let meta = std::fs::metadata(path).map_err(|_| Key::ImageUnreadable)?;
    if !meta.is_file() || meta.len() > MAX_IMAGE_BYTES as u64 {
        return Err(Key::ImageUnreadable);
    }
    let data = std::fs::read(path).map_err(|_| Key::ImageUnreadable)?;
    Ok((mime.into(), data))
}

async fn blocking<T: Send + 'static>(
    fallback: Key,
    job: impl FnOnce() -> Result<T, Key> + Send + 'static,
) -> Result<T, Key> {
    tokio::task::spawn_blocking(job)
        .await
        .unwrap_or(Err(fallback))
}

impl App {
    fn can_paste(&self, i: usize) -> bool {
        !self.running()
            && self.items.get(i).is_some_and(|item| {
                matches!(&item.status, ItemStatus::Failed(e) if e.kind != ScrapeErrorKind::Unsupported)
            })
    }

    pub(super) fn extracting(&self) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item.status, ItemStatus::Extracting))
    }

    pub(super) fn toggle_paste_mode(&mut self, i: usize) {
        if self.can_paste(i) {
            self.paste_row = (self.paste_row != Some(i)).then_some(i);
        }
    }

    fn set_paste_error(&mut self, i: usize, message: Option<String>) {
        if let Some(item) = self.items.get_mut(i) {
            item.paste_error = message;
        }
    }

    fn enter_paste_mode(&mut self, i: usize) -> bool {
        if !self.can_paste(i) {
            return false;
        }
        self.paste_row = Some(i);
        self.set_paste_error(i, None);
        true
    }

    pub(super) fn paste_text(&mut self, i: usize) -> Task<Message> {
        if !self.enter_paste_mode(i) {
            return Task::none();
        }
        clipboard::read().map(move |text| Message::PastedText(i, text))
    }

    pub(super) fn paste_image(&mut self, i: usize) -> Task<Message> {
        if !self.enter_paste_mode(i) {
            return Task::none();
        }
        Task::perform(
            blocking(Key::ClipboardNoImage, read_clipboard_image),
            move |image| Message::PastedImage(i, image),
        )
    }

    pub(super) fn file_dropped(&mut self, path: PathBuf) -> Task<Message> {
        let Some(i) = self.paste_row.filter(|&i| self.can_paste(i)) else {
            return Task::none();
        };
        self.set_paste_error(i, None);
        Task::perform(
            blocking(Key::ImageUnreadable, move || load_image_file(&path)),
            move |image| Message::PastedImage(i, image),
        )
    }

    pub(super) fn pasted_text(&mut self, i: usize, text: Option<String>) -> Task<Message> {
        let text = text.unwrap_or_default();
        let text = text.trim();
        if text.is_empty() {
            return self.paste_failed(i, Key::ClipboardEmpty);
        }
        let clipped: String = text.chars().take(MAX_TEXT_CHARS).collect();
        self.start_extraction(i, vec![Part::Text(clipped)])
    }

    pub(super) fn pasted_image(
        &mut self,
        i: usize,
        image: Result<PastedImage, Key>,
    ) -> Task<Message> {
        match image {
            Ok((mime, data)) => self.start_extraction(i, vec![Part::Image { mime, data }]),
            Err(key) => self.paste_failed(i, key),
        }
    }

    fn paste_failed(&mut self, i: usize, key: Key) -> Task<Message> {
        if self.can_paste(i) {
            let message = t(self.settings.lang, key).to_owned();
            self.set_paste_error(i, Some(message));
        }
        Task::none()
    }

    fn start_extraction(&mut self, i: usize, parts: Vec<Part>) -> Task<Message> {
        if !self.can_paste(i) {
            return Task::none();
        }
        let lang = self.settings.lang;
        let Some(llm) = self.llm.as_ref().map(|h| Arc::clone(&h.client)) else {
            self.set_paste_error(i, Some(t(lang, Key::NoApiKey).to_owned()));
            return Task::none();
        };
        let (site, url) = match classify_url(&self.items[i].url) {
            Ok(classified) => classified,
            Err(e) => {
                self.set_paste_error(i, Some(e.message));
                return Task::none();
            }
        };
        let model = self.settings.advocate_model.clone();
        let item = &mut self.items[i];
        item.status = ItemStatus::Extracting;
        item.paste_error = None;
        Task::perform(
            extract(llm, site, url.to_string(), parts, model),
            move |result| Message::Extracted(i, result),
        )
    }

    pub(super) fn extracted(
        &mut self,
        i: usize,
        result: Result<Box<Product>, String>,
    ) -> Task<Message> {
        let lang = self.settings.lang;
        let stale = self.result.is_some();
        let running = self.running();
        let Some(item) = self.items.get_mut(i) else {
            return Task::none();
        };
        if running || !matches!(item.status, ItemStatus::Extracting | ItemStatus::Failed(_)) {
            return Task::none();
        }
        match result {
            Ok(product) => {
                item.status = ItemStatus::Ready(product);
                item.paste_error = None;
                if self.paste_row == Some(i) {
                    self.paste_row = None;
                }
                self.stale |= stale;
            }
            Err(e) => {
                let message = format!("{}: {e}", t(lang, Key::ExtractFailed));
                item.status =
                    ItemStatus::Failed(ScrapeError::new(ScrapeErrorKind::Layout, message));
            }
        }
        Task::none()
    }

    /// Screenshot fixture: second demo row failed and in paste mode.
    pub(super) fn show_failed_row(&mut self) {
        if let Some(item) = self.items.get_mut(1) {
            item.status = ItemStatus::Failed(ScrapeError::new(
                ScrapeErrorKind::Blocked,
                "temu.com blocked the automated request",
            ));
            item.paste_error = None;
            self.paste_row = Some(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::{Value, json};

    use super::*;
    use crate::demo::demo_comparison;
    use crate::domain::Field;
    use crate::llm::{LlmError, LlmRequest};
    use crate::settings::Settings;
    use crate::ui::ItemState;
    use crate::ui::snap::DevEnv;

    const TEMU_URL: &str = "https://www.temu.com/x-g-601099.html";

    struct FakeLlm {
        reply: Result<Value, LlmError>,
        seen: Mutex<Vec<LlmRequest>>,
    }

    impl Llm for FakeLlm {
        async fn generate_json(&self, req: LlmRequest) -> Result<Value, LlmError> {
            self.seen.lock().unwrap().push(req);
            self.reply.clone()
        }
    }

    fn fake(reply: Result<Value, LlmError>) -> Arc<FakeLlm> {
        Arc::new(FakeLlm {
            reply,
            seen: Mutex::new(Vec::new()),
        })
    }

    fn failed_app() -> App {
        let dev = DevEnv {
            demo: true,
            ..DevEnv::default()
        };
        let mut app = App::new(Settings::default(), None, dev);
        app.items[1].url = TEMU_URL.into();
        app.items[1].status =
            ItemStatus::Failed(ScrapeError::new(ScrapeErrorKind::Blocked, "blocked"));
        app
    }

    fn keyed(mut app: App) -> App {
        app.set_api_key(Some("k".into()));
        app
    }

    fn product() -> Product {
        demo_comparison().1.products[1].clone()
    }

    #[tokio::test]
    async fn extract_keeps_url_site_and_flags_missing() {
        let llm = fake(Ok(json!({
            "title": "Auriculares",
            "price": { "cents": 120000, "currency": "UYU" }
        })));
        let parts = vec![Part::Text("pasted".into())];
        let p = extract(
            llm.clone(),
            Site::Temu,
            TEMU_URL.into(),
            parts.clone(),
            "m".into(),
        )
        .await
        .unwrap();
        assert_eq!((p.site, p.url.as_str()), (Site::Temu, TEMU_URL));
        assert_eq!(p.price.cents, 120_000);
        assert!(p.missing.contains(&Field::Rating));
        let seen = llm.seen.lock().unwrap();
        assert_eq!(seen[0].parts, parts);
        assert_eq!(seen[0].model, "m");
    }

    #[tokio::test]
    async fn extract_reports_llm_and_schema_errors() {
        let auth = fake(Err(LlmError::Auth));
        let e = extract(auth, Site::Temu, TEMU_URL.into(), vec![], "m".into()).await;
        assert!(e.is_err());
        let bad = fake(Ok(json!({ "price_is_range": false })));
        let e = extract(bad, Site::Temu, TEMU_URL.into(), vec![], "m".into()).await;
        assert!(e.is_err());
    }

    #[test]
    fn extracted_ok_becomes_ready_with_the_original_url() {
        let mut app = failed_app();
        app.paste_row = Some(1);
        app.items[1].status = ItemStatus::Extracting;
        let mut p = product();
        p.url = TEMU_URL.into();
        let _ = app.update(Message::Extracted(1, Ok(Box::new(p))));
        let ItemStatus::Ready(p) = &app.items[1].status else {
            panic!("not ready");
        };
        assert_eq!(p.url, TEMU_URL);
        assert_eq!(app.paste_row, None);
        assert!(app.breakdown(1).is_some());
    }

    #[test]
    fn extracted_ok_marks_an_existing_result_stale() {
        let mut app = failed_app();
        app.items[1].status = ItemStatus::Extracting;
        let _ = app.update(Message::Extracted(1, Ok(Box::new(product()))));
        assert!(app.stale);
    }

    #[test]
    fn extracted_error_returns_to_a_failed_pasteable_row() {
        let mut app = failed_app();
        app.items[1].status = ItemStatus::Extracting;
        let _ = app.update(Message::Extracted(1, Err("boom".into())));
        let ItemStatus::Failed(e) = &app.items[1].status else {
            panic!("not failed");
        };
        assert!(e.message.contains("boom"));
        assert!(app.can_paste(1));
    }

    #[test]
    fn late_extraction_for_an_edited_row_is_dropped() {
        let mut app = failed_app();
        app.items[1].status = ItemStatus::Extracting;
        let _ = app.update(Message::UrlChanged(1, "https://other.example".into()));
        let _ = app.update(Message::Extracted(1, Ok(Box::new(product()))));
        assert!(matches!(app.items[1].status, ItemStatus::Idle));
    }

    #[test]
    fn pasted_text_starts_an_extraction() {
        let mut app = keyed(failed_app());
        let text = Some("  Auriculares $ 1.200  ".to_owned());
        let task = app.update(Message::PastedText(1, text));
        assert_eq!(task.units(), 1);
        assert!(matches!(app.items[1].status, ItemStatus::Extracting));
        assert_eq!(app.update(Message::Compare).units(), 0);
        assert_eq!(app.run, crate::ui::RunState::Idle);
    }

    #[test]
    fn blank_clipboard_text_is_reported_without_a_request() {
        let mut app = keyed(failed_app());
        for text in [None, Some("  \n".to_owned())] {
            let task = app.update(Message::PastedText(1, text));
            assert_eq!(task.units(), 0);
            assert!(matches!(app.items[1].status, ItemStatus::Failed(_)));
            assert_eq!(
                app.items[1].paste_error.as_deref(),
                Some(t(app.settings.lang, Key::ClipboardEmpty))
            );
        }
    }

    #[test]
    fn pasting_without_an_api_key_reports_it() {
        let mut app = failed_app();
        let task = app.update(Message::PastedText(1, Some("x".into())));
        assert_eq!(task.units(), 0);
        assert!(matches!(app.items[1].status, ItemStatus::Failed(_)));
        assert!(app.items[1].paste_error.is_some());
    }

    #[test]
    fn pasted_image_starts_an_extraction_and_errors_are_reported() {
        let mut app = keyed(failed_app());
        let task = app.update(Message::PastedImage(1, Err(Key::ClipboardNoImage)));
        assert_eq!(task.units(), 0);
        assert!(app.items[1].paste_error.is_some());
        let image = ("image/png".to_owned(), vec![1, 2]);
        let task = app.update(Message::PastedImage(1, Ok(image)));
        assert_eq!(task.units(), 1);
        assert!(matches!(app.items[1].status, ItemStatus::Extracting));
        assert!(app.items[1].paste_error.is_none());
    }

    #[test]
    fn unsupported_urls_cannot_be_pasted_for() {
        let mut app = keyed(failed_app());
        app.items[1].status = ItemStatus::Failed(ScrapeError::new(
            ScrapeErrorKind::Unsupported,
            "unsupported",
        ));
        assert_eq!(app.update(Message::PasteText(1)).units(), 0);
        let _ = app.update(Message::PasteMode(1));
        assert_eq!(app.paste_row, None);
    }

    #[test]
    fn paste_text_reads_the_clipboard_and_enters_paste_mode() {
        let mut app = keyed(failed_app());
        assert_eq!(app.update(Message::PasteText(1)).units(), 1);
        assert_eq!(app.paste_row, Some(1));
        assert_eq!(app.update(Message::PasteText(0)).units(), 0);
        assert_eq!(app.paste_row, Some(1));
    }

    #[test]
    fn paste_mode_toggles() {
        let mut app = failed_app();
        let _ = app.update(Message::PasteMode(1));
        assert_eq!(app.paste_row, Some(1));
        let _ = app.update(Message::PasteMode(1));
        assert_eq!(app.paste_row, None);
    }

    #[test]
    fn dropped_files_target_only_the_row_in_paste_mode() {
        let mut app = keyed(failed_app());
        let drop = || Message::FileDropped(PathBuf::from("/tmp/shot.png"));
        assert_eq!(app.update(drop()).units(), 0);
        let _ = app.update(Message::PasteMode(1));
        assert_eq!(app.update(drop()).units(), 1);
    }

    #[test]
    fn removing_a_row_keeps_paste_mode_on_the_same_row() {
        let mut app = failed_app();
        app.items.push(ItemState::new(""));
        app.paste_row = Some(2);
        let _ = app.update(Message::RemoveItem(0));
        assert_eq!(app.paste_row, Some(1));
        let _ = app.update(Message::AddItem);
        let _ = app.update(Message::RemoveItem(1));
        assert_eq!(app.paste_row, None);
    }

    #[test]
    fn rows_cannot_be_removed_while_extracting() {
        let mut app = failed_app();
        app.items.push(ItemState::new(""));
        app.items[1].status = ItemStatus::Extracting;
        let _ = app.update(Message::RemoveItem(0));
        assert_eq!(app.items.len(), 3);
    }

    #[test]
    fn image_files_are_validated_by_type_and_size() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("a.PNG");
        std::fs::write(&png, [1, 2, 3]).unwrap();
        assert_eq!(
            load_image_file(&png),
            Ok(("image/png".into(), vec![1, 2, 3]))
        );

        let txt = dir.path().join("a.txt");
        std::fs::write(&txt, "x").unwrap();
        assert_eq!(load_image_file(&txt), Err(Key::ImageUnreadable));

        let big = dir.path().join("big.jpg");
        std::fs::write(&big, vec![0u8; MAX_IMAGE_BYTES + 1]).unwrap();
        assert_eq!(load_image_file(&big), Err(Key::ImageUnreadable));

        let missing = dir.path().join("none.png");
        assert_eq!(load_image_file(&missing), Err(Key::ImageUnreadable));
    }

    #[test]
    fn clipboard_pixels_encode_as_png() {
        let png = encode_png(2, 1, &[255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
        assert_eq!(&png[..4], b"\x89PNG");
        assert!(encode_png(2, 2, &[0; 4]).is_none());
    }
}
