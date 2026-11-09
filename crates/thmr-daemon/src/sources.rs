//! Bounded, unauthenticated audio probes. Never read or forward credentials.
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use thmr_core::settings::Settings;

pub async fn measure(http: &crate::network::PublicHttp, settings: &Settings) -> Vec<Value> {
    let (first, second) = tokio::join!(
        probe(http, settings, "source1", &settings.source1),
        probe(http, settings, "source2", &settings.source2)
    );
    vec![first, second]
}
async fn probe(
    http: &crate::network::PublicHttp,
    settings: &Settings,
    id: &str,
    origin: &str,
) -> Value {
    let start = Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(6), async {
        let url = format!(
            "{}/radio/{}/{}",
            origin.trim_end_matches('/'),
            settings.station_id,
            settings.profile_id
        );
        let mut response = http.get(&url).await?;
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        if !content_type.starts_with("audio/")
            && !content_type.starts_with("application/octet-stream")
        {
            anyhow::bail!("not an audio response");
        }
        let mut bytes = 0;
        while bytes < 32768 {
            let Some(chunk) = response.chunk().await? else {
                break;
            };
            bytes += chunk.len().min(32768 - bytes);
        }
        if bytes < 32768 {
            anyhow::bail!("insufficient audio sample");
        }
        Ok::<_, anyhow::Error>((response.status().as_u16(), content_type, bytes))
    })
    .await;
    match result {
        Ok(Ok((status, content_type, bytes))) => {
            json!({"source":id,"elapsedMs":start.elapsed().as_millis() as u64,"statusCode":status,"contentType":content_type,"bytesRead":bytes})
        }
        _ => {
            json!({"source":id,"elapsedMs":start.elapsed().as_millis() as u64,"bytesRead":0,"error":"audio probe failed or authentication required"})
        }
    }
}

pub fn choose(results: &[Value], current: &str) -> Option<String> {
    let elapsed = |value: &Value| {
        if value.get("bytesRead").and_then(Value::as_u64).unwrap_or(0) >= 32768
            && value.get("error").is_none()
        {
            value.get("elapsedMs").and_then(Value::as_u64)
        } else {
            None
        }
    };
    let fastest = results
        .iter()
        .filter_map(|v| Some((v.get("source")?.as_str()?, elapsed(v)?)))
        .min_by_key(|(_, time)| *time)?;
    let previous = results
        .iter()
        .find(|v| v.get("source").and_then(Value::as_str) == Some(current))
        .and_then(elapsed);
    if fastest.0 == current || previous.is_some_and(|time| fastest.1 as f64 > time as f64 * 0.9) {
        Some(current.into())
    } else {
        Some(fastest.0.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_source_requires_audio_and_meaningful_improvement() {
        let results = vec![
            json!({"source":"source1","bytesRead":32768,"elapsedMs":100}),
            json!({"source":"source2","bytesRead":32768,"elapsedMs":95}),
        ];
        assert_eq!(choose(&results, "source1").as_deref(), Some("source1"));
        let results = vec![
            results[0].clone(),
            json!({"source":"source2","bytesRead":32768,"elapsedMs":50}),
        ];
        assert_eq!(choose(&results, "source1").as_deref(), Some("source2"));
        assert!(choose(
            &[json!({"source":"source1","bytesRead":0,"elapsedMs":1})],
            "source1"
        )
        .is_none());
    }
}
