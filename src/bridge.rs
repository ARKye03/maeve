//! Localhost endpoint for the Maeve browser extension: it posts the HTML of
//! shop pages the user opens, so pages that block the scraper can still be read.

use std::net::{Ipv4Addr, SocketAddr};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::post;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::scrape::classify_url;

pub const PORT: u16 = 47821;
const MAX_BODY: usize = 16 * 1024 * 1024;
/// Web pages cannot send a custom header to another origin without a CORS
/// preflight, which this server never answers; extensions with host
/// permission can.
const EXTENSION_HEADER: &str = "x-maeve-extension";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub url: String,
    pub html: String,
}

#[derive(Deserialize)]
struct Payload {
    url: String,
    html: String,
}

pub async fn listen() -> std::io::Result<TcpListener> {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, PORT))).await
}

pub async fn serve(listener: TcpListener, tx: mpsc::Sender<Page>) -> std::io::Result<()> {
    let port = listener.local_addr()?.port();
    let app = Router::new()
        .route("/page", post(page))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state((tx, port));
    axum::serve(listener, app).await
}

async fn page(
    State((tx, port)): State<(mpsc::Sender<Page>, u16)>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    match accept(&headers, port, &body) {
        Ok(page) => match tx.try_send(page) {
            Ok(()) => StatusCode::ACCEPTED,
            Err(_) => StatusCode::SERVICE_UNAVAILABLE,
        },
        Err(status) => status,
    }
}

fn accept(headers: &HeaderMap, port: u16, body: &[u8]) -> Result<Page, StatusCode> {
    if !trusted(headers, port) {
        return Err(StatusCode::FORBIDDEN);
    }
    let payload: Payload = serde_json::from_slice(body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let (_, url) = classify_url(&payload.url).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    Ok(Page {
        url: url.to_string(),
        html: payload.html,
    })
}

fn trusted(headers: &HeaderMap, port: u16) -> bool {
    let text = |name| headers.get(name).and_then(|v| v.to_str().ok());
    // Host check defeats DNS rebinding: a rebound page would send its own host name.
    let host_ok = text(header::HOST.as_str())
        .is_some_and(|h| h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"));
    let origin_ok = text(header::ORIGIN.as_str()).is_none_or(|o| {
        ["chrome-extension://", "moz-extension://"]
            .iter()
            .any(|p| o.starts_with(p))
    });
    host_ok && origin_ok && text(EXTENSION_HEADER) == Some("1")
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    const URL: &str = "https://www.temu.com/uy/cesta-g-601100900000000.html";

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_static(v));
        }
        h
    }

    fn body(url: &str) -> Vec<u8> {
        serde_json::json!({ "url": url, "html": "<html></html>" })
            .to_string()
            .into_bytes()
    }

    const GOOD: &[(&str, &str)] = &[
        ("host", "127.0.0.1:47821"),
        ("origin", "chrome-extension://abc"),
        ("x-maeve-extension", "1"),
    ];

    #[test]
    fn extension_requests_are_accepted() {
        let page = accept(&headers(GOOD), PORT, &body(URL)).unwrap();
        assert_eq!(page.url, URL);
        let firefox = headers(&[
            ("host", "localhost:47821"),
            ("origin", "moz-extension://uuid"),
            ("x-maeve-extension", "1"),
        ]);
        assert!(accept(&firefox, PORT, &body(URL)).is_ok());
    }

    #[test]
    fn web_pages_and_rebound_hosts_are_refused() {
        let cases: [&[(&str, &str)]; 3] = [
            &[
                ("host", "127.0.0.1:47821"),
                ("origin", "https://evil.example"),
                ("x-maeve-extension", "1"),
            ],
            &[("host", "evil.example:47821"), ("x-maeve-extension", "1")],
            &[
                ("host", "127.0.0.1:47821"),
                ("origin", "chrome-extension://abc"),
            ],
        ];
        for case in cases {
            assert_eq!(
                accept(&headers(case), PORT, &body(URL)),
                Err(StatusCode::FORBIDDEN)
            );
        }
    }

    #[test]
    fn bad_bodies_and_other_sites_are_rejected() {
        let h = headers(GOOD);
        assert_eq!(accept(&h, PORT, b"nope"), Err(StatusCode::BAD_REQUEST));
        assert_eq!(
            accept(&h, PORT, &body("https://amazon.com/x")),
            Err(StatusCode::UNPROCESSABLE_ENTITY)
        );
    }

    #[tokio::test]
    async fn pages_posted_over_http_reach_the_channel() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, mut rx) = mpsc::channel(1);
        tokio::spawn(serve(listener, tx));

        let status = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/page"))
            .header("x-maeve-extension", "1")
            .body(body(URL))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, StatusCode::ACCEPTED.as_u16());
        assert_eq!(rx.recv().await.unwrap().url, URL);
    }
}
