use axum::{
    extract::{ws::Message, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use std::time::Duration;
use thmr_core::{
    protocol::{ApiError, Request, Snapshot},
    PROTOCOL_VERSION,
};
use tokio::sync::{mpsc, oneshot, watch};

pub struct Envelope {
    pub request: Request,
    pub reply: oneshot::Sender<Result<Snapshot, String>>,
}
#[derive(Clone)]
pub struct AppState {
    pub token: String,
    pub commands: mpsc::Sender<Envelope>,
    pub snapshots: watch::Receiver<Snapshot>,
}

fn error(code: StatusCode, message: impl Into<String>) -> Response {
    (
        code,
        Json(ApiError {
            error: message.into(),
        }),
    )
        .into_response()
}

async fn authorize(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    // Native loopback clients do not send Origin. Reject browser requests, including null Origin.
    if request.headers().contains_key("origin") {
        return error(StatusCode::FORBIDDEN, "browser access is disabled");
    }
    let auth = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let expected = format!("Bearer {}", state.token);
    if auth.len() != expected.len()
        || auth
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            != 0
    {
        return error(StatusCode::UNAUTHORIZED, "authentication required");
    }
    next.run(request).await
}

fn compatible(headers: &HeaderMap) -> bool {
    headers
        .get("x-thmr-protocol")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.parse::<u32>().ok())
        == Some(PROTOCOL_VERSION)
}

async fn snapshot(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !compatible(&headers) {
        return error(StatusCode::CONFLICT, "incompatible THMR protocol");
    }
    Json(state.snapshots.borrow().clone()).into_response()
}

async fn command(State(state): State<AppState>, Json(request): Json<Request>) -> Response {
    if request.protocol_version != PROTOCOL_VERSION {
        return error(StatusCode::CONFLICT, "incompatible THMR protocol");
    }
    let (reply, receive) = oneshot::channel();
    if state
        .commands
        .send(Envelope { request, reply })
        .await
        .is_err()
    {
        return error(StatusCode::SERVICE_UNAVAILABLE, "host is stopping");
    }
    match tokio::time::timeout(Duration::from_secs(4), receive).await {
        Ok(Ok(Ok(state))) => Json(state).into_response(),
        Ok(Ok(Err(e))) => error(StatusCode::BAD_REQUEST, e),
        _ => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "host did not confirm command",
        ),
    }
}

async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !compatible(&headers) {
        return error(StatusCode::CONFLICT, "incompatible THMR protocol");
    }
    ws.on_upgrade(move |mut socket| async move {
        let mut snapshots = state.snapshots.clone();
        loop {
            let snapshot = snapshots.borrow_and_update().clone();
            let Ok(text) = serde_json::to_string(&snapshot) else { break; };
            if socket.send(Message::Text(text.into())).await.is_err() { break; }
            loop {
                tokio::select! {
                    changed = snapshots.changed() => { if changed.is_err() { return; } break; }
                    incoming = socket.next() => match incoming {
                        Some(Ok(Message::Ping(bytes))) => { if socket.send(Message::Pong(bytes)).await.is_err() { return; } }
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                        _ => {}
                    }
                }
            }
        }
    }).into_response()
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/state", get(snapshot))
        .route("/v1/command", post(command))
        .route("/v1/events", get(events))
        .layer(axum::extract::DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn_with_state(state.clone(), authorize))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn control_endpoint_authenticates_http_and_upgrade_and_rejects_browser_and_version() {
        let (commands, _rx) = mpsc::channel(4);
        let (_tx, snapshots) = watch::channel(Snapshot::new("test".into(), &Default::default()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let host = tokio::spawn(async move {
            axum::serve(
                listener,
                router(AppState {
                    token: "secret".into(),
                    commands,
                    snapshots,
                }),
            )
            .await
            .unwrap();
        });
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        assert_eq!(
            http.get(format!("{base}/v1/state"))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            http.get(format!("{base}/v1/events"))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            http.get(format!("{base}/v1/state"))
                .bearer_auth("secret")
                .header("Origin", "http://localhost")
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        assert_eq!(
            http.get(format!("{base}/v1/state"))
                .bearer_auth("secret")
                .header("X-THMR-Protocol", "999")
                .send()
                .await
                .unwrap()
                .status(),
            409
        );
        assert_eq!(
            http.get(format!("{base}/v1/state"))
                .bearer_auth("secret")
                .header("X-THMR-Protocol", "1")
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        host.abort();
    }
}
