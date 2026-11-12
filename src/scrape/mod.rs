mod sidecar;

use std::future::Future;

use thiserror::Error;
use url::Url;

use crate::domain::{Product, Site};

pub use sidecar::{SidecarCommand, SidecarScraper};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrapeErrorKind {
    Blocked,
    Layout,
    Network,
    Unsupported,
    Timeout,
    SidecarUnavailable,
}

#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct ScrapeError {
    pub kind: ScrapeErrorKind,
    pub message: String,
}

impl ScrapeError {
    pub fn new(kind: ScrapeErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

pub trait Scraper: Send + Sync + 'static {
    fn scrape(&self, url: Url) -> impl Future<Output = Result<Product, ScrapeError>> + Send;
}

const ML_DOMAIN: &str = "mercadolibre.com.uy";
const TEMU_DOMAIN: &str = "temu.com";

fn host_matches(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|rest| rest.ends_with('.'))
}

/// Validates the URL against the supported shops and drops every query
/// parameter except the ones that select a product variant.
pub fn classify_url(raw: &str) -> Result<(Site, Url), ScrapeError> {
    let unsupported = || {
        let shown: String = raw.chars().take(80).collect();
        ScrapeError::new(
            ScrapeErrorKind::Unsupported,
            format!("unsupported url: {shown:?}"),
        )
    };
    let raw = raw.trim();
    if raw.is_empty()
        || raw.contains('\\')
        || raw.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(unsupported());
    }
    let mut url = Url::parse(raw).map_err(|_| unsupported())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(unsupported());
    }
    let host = url.host_str().ok_or_else(unsupported)?.to_ascii_lowercase();
    let (site, variant_keys): (Site, &[&str]) = if host_matches(&host, ML_DOMAIN) {
        (Site::MercadoLibre, &["variation"])
    } else if host_matches(&host, TEMU_DOMAIN) {
        (Site::Temu, &["sku_id", "skuId"])
    } else {
        return Err(unsupported());
    };

    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| variant_keys.contains(&key.as_ref()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        url.set_query(None);
    } else {
        url.query_pairs_mut().clear().extend_pairs(kept);
    }
    // Mercado Libre also carries the selected variation in the fragment.
    if site == Site::Temu {
        url.set_fragment(None);
    }
    Ok((site, url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_mercadolibre_uy() {
        let (site, url) =
            classify_url("https://articulo.mercadolibre.com.uy/MLU-123-x-_JM").unwrap();
        assert_eq!(site, Site::MercadoLibre);
        assert_eq!(
            url.as_str(),
            "https://articulo.mercadolibre.com.uy/MLU-123-x-_JM"
        );
    }

    #[test]
    fn strips_temu_tracking_params() {
        let (site, url) =
            classify_url("https://www.temu.com/x-g-601099.html?_x_sessn_id=1&utm_source=a")
                .unwrap();
        assert_eq!(site, Site::Temu);
        assert_eq!(url.as_str(), "https://www.temu.com/x-g-601099.html");
    }

    #[test]
    fn keeps_variant_ids() {
        let (_, url) =
            classify_url("https://www.temu.com/x-g-1.html?sku_id=77&_x_sessn_id=1").unwrap();
        assert_eq!(url.query(), Some("sku_id=77"));
        let (_, url) = classify_url(
            "https://articulo.mercadolibre.com.uy/MLU-1-x-_JM?variation=9&tracking_id=z#variation=9",
        )
        .unwrap();
        assert_eq!(url.query(), Some("variation=9"));
        assert_eq!(url.fragment(), Some("variation=9"));
    }

    #[test]
    fn rejects_unsupported_hosts() {
        for raw in [
            "https://amazon.com/x",
            "https://articulo.mercadolibre.com.ar/MLA-1-x",
            "https://evilmercadolibre.com.uy/x",
            "https://mercadolibre.com.uy.evil.com/x",
            "https://user@www.temu.com/x",
            "https://www.temu.com:8443/x",
            "ftp://www.temu.com/x",
            "not a url",
            "",
        ] {
            let err = classify_url(raw).unwrap_err();
            assert_eq!(err.kind, ScrapeErrorKind::Unsupported, "{raw}");
        }
    }
}
