use std::time::Duration;

use serde::Deserialize;

use crate::domain::FxRate;

pub const DEFAULT_BASE_URL: &str = "https://open.er-api.com";

/// Sanity band for UYU per USD; anything outside is a typo or a broken feed.
pub const MIN_UYU_PER_USD: f64 = 1.0;
pub const MAX_UYU_PER_USD: f64 = 1_000.0;

const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq)]
pub enum RatePick {
    Use(FxRate),
    Fetch,
}

/// Rejects non-finite, non-positive and absurd rates so a bad value can never
/// silently zero out converted prices.
pub fn validate_rate(rate: f64) -> Result<f64, String> {
    if !rate.is_finite() {
        return Err(format!("exchange rate must be a finite number, got {rate}"));
    }
    if !(MIN_UYU_PER_USD..=MAX_UYU_PER_USD).contains(&rate) {
        return Err(format!(
            "exchange rate {rate} is outside the plausible range {MIN_UYU_PER_USD}..={MAX_UYU_PER_USD} UYU per USD"
        ));
    }
    Ok(rate)
}

pub fn validate_fx(fx: &FxRate) -> Result<(), String> {
    validate_rate(fx.uyu_per_usd).map(|_| ())
}

/// Precedence: valid manual > today's valid auto-fetched cache > fetch.
/// An invalid manual value is ignored here; validate user input with
/// [`validate_rate`] to report it.
pub fn pick_rate(
    cached: Option<FxRate>,
    today: jiff::civil::Date,
    manual: Option<f64>,
) -> RatePick {
    if let Some(rate) = manual.and_then(|m| validate_rate(m).ok()) {
        return RatePick::Use(FxRate {
            uyu_per_usd: rate,
            date: today,
            manual: true,
        });
    }
    match cached {
        Some(c) if c.date == today && !c.manual && validate_fx(&c).is_ok() => RatePick::Use(c),
        _ => RatePick::Fetch,
    }
}

#[derive(Deserialize)]
struct RatesResponse {
    rates: Rates,
}

#[derive(Deserialize)]
struct Rates {
    #[serde(rename = "UYU")]
    uyu: f64,
}

/// `base_url` is the API origin, e.g. [`DEFAULT_BASE_URL`].
pub async fn fetch_rate(http: &reqwest::Client, base_url: &str) -> Result<FxRate, String> {
    let url = format!("{}/v6/latest/USD", base_url.trim_end_matches('/'));
    let resp = http
        .get(&url)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("exchange rate request failed: {}", e.without_url()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("exchange rate service returned {status}"));
    }
    let body: RatesResponse = resp
        .json()
        .await
        .map_err(|e| format!("exchange rate response is invalid: {}", e.without_url()))?;
    let uyu_per_usd = validate_rate(body.rates.uyu)?;
    Ok(FxRate {
        uyu_per_usd,
        date: jiff::Zoned::now().date(),
        manual: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn today() -> jiff::civil::Date {
        jiff::civil::date(2026, 11, 12)
    }

    fn cached(date: jiff::civil::Date, rate: f64) -> FxRate {
        FxRate {
            uyu_per_usd: rate,
            date,
            manual: false,
        }
    }

    #[test]
    fn validate_rejects_bad_values() {
        for bad in [
            0.0,
            -40.0,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.5,
            1_000.1,
            4_012.0,
        ] {
            assert!(validate_rate(bad).is_err(), "{bad}");
        }
        for ok in [1.0, 40.12, 1_000.0] {
            assert_eq!(validate_rate(ok), Ok(ok));
        }
    }

    #[test]
    fn manual_wins_over_cache() {
        let pick = pick_rate(Some(cached(today(), 39.0)), today(), Some(41.5));
        assert_eq!(
            pick,
            RatePick::Use(FxRate {
                uyu_per_usd: 41.5,
                date: today(),
                manual: true
            })
        );
    }

    #[test]
    fn todays_cache_is_used() {
        let c = cached(today(), 39.0);
        assert_eq!(pick_rate(Some(c.clone()), today(), None), RatePick::Use(c));
    }

    #[test]
    fn stale_or_missing_cache_fetches() {
        let yesterday = today().yesterday().unwrap();
        assert_eq!(
            pick_rate(Some(cached(yesterday, 39.0)), today(), None),
            RatePick::Fetch
        );
        assert_eq!(pick_rate(None, today(), None), RatePick::Fetch);
    }

    #[test]
    fn invalid_manual_is_ignored() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e9] {
            assert_eq!(
                pick_rate(None, today(), Some(bad)),
                RatePick::Fetch,
                "{bad}"
            );
            let c = cached(today(), 39.0);
            assert_eq!(
                pick_rate(Some(c.clone()), today(), Some(bad)),
                RatePick::Use(c),
                "{bad}"
            );
        }
    }

    #[test]
    fn invalid_cache_fetches() {
        for bad in [0.0, -3.0, f64::NAN] {
            assert_eq!(
                pick_rate(Some(cached(today(), bad)), today(), None),
                RatePick::Fetch,
                "{bad}"
            );
        }
    }

    #[test]
    fn cleared_manual_override_is_not_reused_from_cache() {
        let c = FxRate {
            manual: true,
            ..cached(today(), 41.5)
        };
        assert_eq!(pick_rate(Some(c), today(), None), RatePick::Fetch);
    }

    async fn serve(status: u16, body: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v6/latest/USD"))
            .respond_with(ResponseTemplate::new(status).set_body_raw(body, "application/json"))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn fetches_uyu_rate() {
        let server = serve(200, r#"{"rates":{"UYU":40.12}}"#).await;
        let rate = fetch_rate(&reqwest::Client::new(), &server.uri())
            .await
            .unwrap();
        assert_eq!(rate.uyu_per_usd, 40.12);
        assert!(!rate.manual);
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_rates() {
        for body in [
            r#"{"rates":{"UYU":0}}"#,
            r#"{"rates":{"UYU":-5.0}}"#,
            r#"{"rates":{"UYU":999999}}"#,
            r#"{"rates":{"EUR":0.9}}"#,
            r#"{"rates":{"UYU":"40"}}"#,
            "not json",
        ] {
            let server = serve(200, body).await;
            assert!(
                fetch_rate(&reqwest::Client::new(), &server.uri())
                    .await
                    .is_err(),
                "{body}"
            );
        }
    }

    #[tokio::test]
    async fn fetch_reports_http_errors() {
        let server = serve(500, "{}").await;
        let err = fetch_rate(&reqwest::Client::new(), &server.uri())
            .await
            .unwrap_err();
        assert!(err.contains("500"), "{err}");
    }
}
