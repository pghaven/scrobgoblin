use axum::{
    extract::{Multipart, State},
    http::StatusCode,
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
        .route("/1/submit-listens", post(navidrome_handler))
        .route("/webhooks/plex", post(plex_handler))
        .route("/webhooks/jellyfin", post(jellyfin_handler))
        .with_state(state)
}

async fn validate_token_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "code": 200,
        "message": "Token valid.",
        "valid": true,
        "user_name": "scroblin"
    }))
}

async fn navidrome_handler(
    State(state): State<AppState>,
    Json(body): Json<sources::navidrome::LbPayload>,
) -> StatusCode {
    match sources::navidrome::parse(&body) {
        Ok(event) if threshold::qualifies(&event) => {
            tokio::spawn(targets::fan_out(state.cfg, state.client, event));
            StatusCode::OK
        }
        Ok(_) => StatusCode::OK,
        Err(e) => {
            eprintln!("[WARN] Navidrome parse error: {}", e);
            StatusCode::BAD_REQUEST
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
