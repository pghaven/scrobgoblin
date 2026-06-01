use axum::{
    extract::{Multipart, Request, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

use crate::{config::Config, sources, targets, threshold};

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub client: reqwest::Client,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        // Navidrome uses ND_LISTENBRAINZ_BASEURL and calls the real LB API paths
        .route("/validate-token", get(validate_token_handler))
        .route("/1/validate-token", get(validate_token_handler))
        .route("/submit-listens", post(navidrome_handler))
        .route("/1/submit-listens", post(navidrome_handler))
        .route("/webhooks/plex", post(plex_handler))
        .route("/webhooks/jellyfin", post(jellyfin_handler))
        .fallback(unmatched_handler)
        .with_state(state)
}

async fn unmatched_handler(req: Request) -> StatusCode {
    eprintln!("[404] {} {}", req.method(), req.uri().path());
    StatusCode::NOT_FOUND
}

async fn validate_token_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !authorized(&headers, &state.cfg.server.webhook_token) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({
            "code": 401,
            "message": "Token invalid.",
            "valid": false
        }))).into_response();
    }
    println!("[REQ] GET /validate-token");
    (StatusCode::OK, Json(serde_json::json!({
        "code": 200,
        "message": "Token valid.",
        "valid": true,
        "user_name": "scroblin"
    }))).into_response()
}

/// Returns true if the request is authorized.
/// If `webhook_token` is None (not configured) every request is allowed —
/// suitable for internal-only deployments. If it is set, the request must
/// carry `Authorization: Token <webhook_token>`.
fn authorized(headers: &HeaderMap, expected: &Option<String>) -> bool {
    let Some(expected_token) = expected else {
        return true;
    };
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Token "))
        .unwrap_or("");
    provided == expected_token.as_str()
}

fn lb_ok() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

async fn navidrome_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<sources::navidrome::LbPayload>,
) -> impl IntoResponse {
    if !authorized(&headers, &state.cfg.server.webhook_token) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"status": "error", "error": "invalid token"}))).into_response();
    }
    if body.listen_type.as_deref() == Some("playing_now") {
        match sources::navidrome::parse_now_playing(&body) {
            Ok(event) => {
                println!("[REQ] playing_now | {} - {}", event.artist, event.track);
                tokio::spawn(targets::fan_out_now_playing(state.cfg, state.client, event));
            }
            Err(e) => eprintln!("[WARN] Navidrome now-playing parse error: {}", e),
        }
        return lb_ok().into_response();
    }
    let ts_info = body.payload.first()
        .map(|l| format!(" listened_at={}", l.listened_at.map(|t| t.to_string()).unwrap_or_else(|| "none".into())))
        .unwrap_or_default();
    println!("[REQ] POST /1/submit-listens ({} listen(s)){}",  body.payload.len(), ts_info);
    match sources::navidrome::parse(&body) {
        Ok(event) if threshold::qualifies(&event) => {
            tokio::spawn(targets::fan_out(state.cfg, state.client, event));
            lb_ok().into_response()
        }
        Ok(_) => lb_ok().into_response(),
        Err(e) => {
            eprintln!("[WARN] Navidrome parse error: {}", e);
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"status": "error"}))).into_response()
        }
    }
}

async fn plex_handler(State(state): State<AppState>, mut multipart: Multipart) -> StatusCode {
    let mut payload_json: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() == Some("payload") {
            if let Ok(text) = field.text().await {
                payload_json = Some(text);
                break;
            }
        }
    }

    let json_str = match payload_json {
        Some(s) => s,
        None => {
            eprintln!("[WARN] Plex webhook missing payload field");
            return StatusCode::BAD_REQUEST;
        }
    };

    let plex_payload = match serde_json::from_str::<sources::plex::PlexPayload>(&json_str) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[WARN] Plex JSON parse error: {}", e);
            return StatusCode::BAD_REQUEST;
        }
    };

    match sources::plex::parse(&plex_payload) {
        Ok(event) if threshold::qualifies(&event) => {
            tokio::spawn(targets::fan_out(state.cfg, state.client, event));
            StatusCode::OK
        }
        Ok(_) => StatusCode::OK,
        Err(e) if e.to_string().contains("not a scrobble event") => StatusCode::OK,
        Err(e) => {
            eprintln!("[WARN] Plex parse error: {}", e);
            StatusCode::BAD_REQUEST
        }
    }
}

async fn jellyfin_handler(
    State(state): State<AppState>,
    Json(body): Json<sources::jellyfin::JellyfinPayload>,
) -> StatusCode {
    match sources::jellyfin::parse(&body) {
        Ok(event) if threshold::qualifies(&event) => {
            tokio::spawn(targets::fan_out(state.cfg, state.client, event));
            StatusCode::OK
        }
        Ok(_) => StatusCode::OK,
        Err(e) if e.to_string().contains("not a PlaybackStopped event") => StatusCode::OK,
        Err(e) => {
            eprintln!("[WARN] Jellyfin parse error: {}", e);
            StatusCode::BAD_REQUEST
        }
    }
}
