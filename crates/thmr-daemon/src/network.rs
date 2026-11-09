//! Shared pacing/cooldown for metadata and source probes. Never retry a request.
use anyhow::{bail, Result};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone)]
pub struct PublicHttp {
    client: reqwest::Client,
    hosts: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<Host>>>>>,
    spacing: Duration,
}

#[derive(Default)]
struct Host {
    next_start: Option<Instant>,
    blocked_until: Option<Instant>,
    failures: u32,
}

fn retry_after(value: &str) -> Option<Duration> {
    value
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
        .or_else(|| {
            httpdate::parse_http_date(value)
                .ok()
                .map(|date| date.duration_since(SystemTime::now()).unwrap_or_default())
        })
}

impl Host {
    fn failed(&mut self, status: Option<u16>, retry: Option<Duration>) {
        self.failures = (self.failures + 1).min(6);
        let mut delay = Duration::from_secs((10 * (1 << (self.failures - 1))).min(300));
        if matches!(status, Some(403 | 429)) {
            delay = delay.max(Duration::from_secs(60));
        }
        delay = delay.max(retry.unwrap_or_default());
        self.blocked_until = Some(Instant::now() + delay);
    }
}

impl PublicHttp {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(8))
                .user_agent("THMR-Client/1.0 (+https://thmr.yunmoan.cn)")
                .build()?,
            hosts: Arc::default(),
            spacing: Duration::from_millis(500),
        })
    }

    pub async fn get(&self, url: &str) -> Result<reqwest::Response> {
        let url = reqwest::Url::parse(url)?;
        let host = {
            let mut hosts = self.hosts.lock().expect("HTTP host map");
            hosts
                .entry(url.origin().ascii_serialization())
                .or_default()
                .clone()
        };
        let mut host = host.lock().await;
        if host
            .blocked_until
            .is_some_and(|until| until > Instant::now())
        {
            bail!("public service cooling down; request not sent");
        }
        if let Some(at) = host.next_start {
            tokio::time::sleep(at.saturating_duration_since(Instant::now())).await;
        }
        host.next_start = Some(Instant::now() + self.spacing);
        let response = match self.client.get(url).send().await {
            Ok(response) => response,
            Err(error) => {
                host.failed(None, None);
                return Err(error.into());
            }
        };
        let status = response.status().as_u16();
        if matches!(status, 403 | 429 | 500..=599) {
            let retry = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(retry_after);
            host.failed(Some(status), retry);
        } else if response.status().is_success() {
            host.failures = 0;
            host.blocked_until = None;
        }
        Ok(response.error_for_status()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn rejection_stops_other_paths_and_cloned_clients() {
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let router = Router::new().fallback(get(move || {
            count.fetch_add(1, Ordering::SeqCst);
            async {
                (
                    axum::http::StatusCode::TOO_MANY_REQUESTS,
                    [("retry-after", "120")],
                )
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let http = PublicHttp::new().unwrap();
        assert!(http.get(&format!("{base}/status")).await.is_err());
        let clone = http.clone();
        for _ in 0..100 {
            assert!(clone.get(&format!("{base}/radio")).await.is_err());
        }
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[test]
    fn cooldown_obeys_both_retry_after_formats_and_backs_off() {
        assert_eq!(retry_after("120"), Some(Duration::from_secs(120)));
        let future = SystemTime::now() + Duration::from_secs(120);
        assert!(retry_after(&httpdate::fmt_http_date(future)).unwrap() > Duration::from_secs(118));
        assert!(retry_after("invalid").is_none());
        let mut host = Host::default();
        host.failed(None, None);
        let first = host.blocked_until.unwrap();
        host.failed(None, None);
        assert!(host.blocked_until.unwrap() > first + Duration::from_secs(9));
        host.failed(Some(429), Some(Duration::from_secs(180)));
        assert!(host.blocked_until.unwrap() > Instant::now() + Duration::from_secs(179));
    }
}
