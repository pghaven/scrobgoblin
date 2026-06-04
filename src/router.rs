use axum::{
    extract::{Multipart, Path, Request, State},
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
        .route("/webhooks/plex/{token}", post(plex_handler))
        .route("/webhooks/plex", post(plex_missing_token_handler))
        .route("/webhooks/jellyfin", post(jellyfin_handler))
        .fallback(unmatched_handler)
        .with_state(state)
}

async fn unmatched_handler(req: Request) -> StatusCode {
    let path = req.uri().path();
    // Scrub the token segment from Plex webhook URLs to avoid logging credentials.
    let safe_path = if path.starts_with("/webhooks/plex/") {
        "/webhooks/plex/<token>"
    } else {
        path
    };
    eprintln!("[404] {} {}", req.method(), safe_path);
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

fn token_matches(expected: Option<&str>, provided: &str) -> bool {
    match expected {
        None | Some("") => true, // not configured or misconfigured empty string — open
        Some(t) => t == provided,
    }
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

async fn plex_missing_token_handler() -> StatusCode {
    eprintln!("[WARN] Plex webhook received at /webhooks/plex — update URL to /webhooks/plex/{{token}}");
    StatusCode::NOT_FOUND
}

async fn plex_handler(
    State(state): State<AppState>,
    Path(url_token): Path<String>,
    mut multipart: Multipart,
) -> StatusCode {
    if !token_matches(state.cfg.plex.webhook_token.as_deref(), &url_token) {
        eprintln!("[WARN] Plex auth failed");
        return StatusCode::UNAUTHORIZED;
    }

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

    match plex_payload.event.as_str() {
        "media.play" | "media.resume" => {
            match sources::plex::parse_now_playing(&plex_payload) {
                Ok(event) => {
                    println!("[REQ] playing_now (plex) | {} - {}", event.artist, event.track);
                    tokio::spawn(targets::fan_out_now_playing(state.cfg, state.client, event));
                }
                Err(e) => eprintln!("[WARN] Plex now-playing parse error: {}", e),
            }
            StatusCode::OK
        }
        "media.scrobble" => {
            match sources::plex::parse(&plex_payload) {
                Ok(event) if threshold::qualifies(&event) => {
                    tokio::spawn(targets::fan_out(state.cfg, state.client, event));
                }
                Ok(_) => {}
                Err(e) => eprintln!("[WARN] Plex scrobble parse error: {}", e),
            }
            StatusCode::OK
        }
        _ => StatusCode::OK,
    }
}

async fn jellyfin_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<sources::jellyfin::JellyfinPayload>,
) -> StatusCode {
    let provided = headers
        .get("x-scroblin-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !token_matches(state.cfg.jellyfin.webhook_token.as_deref(), provided) {
        eprintln!("[WARN] Jellyfin auth failed");
        return StatusCode::UNAUTHORIZED;
    }

    match body.notification_type.as_str() {
        "PlaybackStart" => {
            match sources::jellyfin::parse_now_playing(&body) {
                Ok(event) => {
                    println!("[REQ] playing_now (jellyfin) | {} - {}", event.artist, event.track);
                    tokio::spawn(targets::fan_out_now_playing(state.cfg, state.client, event));
                }
                Err(e) => eprintln!("[WARN] Jellyfin now-playing parse error: {}", e),
            }
            StatusCode::OK
        }
        "PlaybackStop" => {
            match sources::jellyfin::parse(&body) {
                Ok(event) if threshold::qualifies(&event) => {
                    tokio::spawn(targets::fan_out(state.cfg, state.client, event));
                }
                Ok(_) => {}
                Err(e) if e.to_string().contains("position 0") => {}
                Err(e) => eprintln!("[WARN] Jellyfin scrobble parse error: {}", e),
            }
            StatusCode::OK
        }
        _ => StatusCode::OK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    fn test_app_plex_token(plex_token: Option<&str>) -> Router {
        let cfg = Arc::new(Config {
            server: crate::config::ServerConfig { port: 4567, webhook_token: None },
            plex: crate::config::PlexConfig {
                webhook_token: plex_token.map(|s| s.to_string()),
            },
            jellyfin: crate::config::JellyfinConfig { webhook_token: None },
            koito: crate::config::KoitoConfig {
                base_url: "http://k".into(),
                api_key: "k".into(),
                forward_now_playing: None,
            },
            listenbrainz: crate::config::ListenBrainzConfig {
                user_token: "t".into(),
                forward_now_playing: None,
            },
            lastfm: crate::config::LastFmConfig {
                api_key: "a".into(),
                shared_secret: "s".into(),
                session_key: "k".into(),
                forward_now_playing: None,
            },
        });
        build_router(AppState { cfg, client: reqwest::Client::new() })
    }

    #[tokio::test]
    async fn plex_handler_rejects_wrong_url_token() {
        let app = test_app_plex_token(Some("secret"));
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/webhooks/plex/wrong")
                    .header("content-type", "multipart/form-data; boundary=----boundary")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn plex_handler_allows_when_no_token_configured() {
        let app = test_app_plex_token(None);
        // With no token configured, auth passes regardless of URL segment.
        // Multipart will be malformed so we get 400, but NOT 401.
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/webhooks/plex/anything")
                    .header("content-type", "multipart/form-data; boundary=----boundary")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn plex_handler_accepts_correct_url_token() {
        let app = test_app_plex_token(Some("secret"));
        // Correct token — auth passes. Multipart is malformed → 400, but NOT 401.
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/webhooks/plex/secret")
                    .header("content-type", "multipart/form-data; boundary=----boundary")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    }

    fn test_app_jellyfin_token(jellyfin_token: Option<&str>) -> Router {
        let cfg = Arc::new(Config {
            server: crate::config::ServerConfig { port: 4567, webhook_token: None },
            plex: crate::config::PlexConfig { webhook_token: None },
            jellyfin: crate::config::JellyfinConfig {
                webhook_token: jellyfin_token.map(|s| s.to_string()),
            },
            koito: crate::config::KoitoConfig {
                base_url: "http://k".into(),
                api_key: "k".into(),
                forward_now_playing: None,
            },
            listenbrainz: crate::config::ListenBrainzConfig {
                user_token: "t".into(),
                forward_now_playing: None,
            },
            lastfm: crate::config::LastFmConfig {
                api_key: "a".into(),
                shared_secret: "s".into(),
                session_key: "k".into(),
                forward_now_playing: None,
            },
        });
        build_router(AppState { cfg, client: reqwest::Client::new() })
    }

    #[tokio::test]
    async fn jellyfin_handler_rejects_wrong_header_token() {
        let app = test_app_jellyfin_token(Some("secret"));
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/webhooks/jellyfin")
                    .header("content-type", "application/json")
                    .header("x-scroblin-token", "wrong")
                    .body(Body::from(r#"{"NotificationType":"PlaybackStopped","Name":"Track"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn jellyfin_handler_rejects_missing_header_when_token_configured() {
        let app = test_app_jellyfin_token(Some("secret"));
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/webhooks/jellyfin")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"NotificationType":"PlaybackStopped","Name":"Track"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn jellyfin_handler_allows_when_no_token_configured() {
        let app = test_app_jellyfin_token(None);
        // No token configured — auth passes regardless of header presence.
        // Valid minimal payload so deserialization succeeds and we get 200.
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/webhooks/jellyfin")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"NotificationType":"PlaybackStopped","Name":"Track"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn token_matches_allows_when_no_expected_token() {
        assert!(token_matches(None, "anything"));
        assert!(token_matches(None, ""));
    }

    #[test]
    fn token_matches_allows_when_tokens_match() {
        assert!(token_matches(Some("secret"), "secret"));
    }

    #[test]
    fn token_matches_rejects_when_tokens_differ() {
        assert!(!token_matches(Some("secret"), "wrong"));
        assert!(!token_matches(Some("secret"), ""));
    }

    #[test]
    fn token_matches_allows_when_empty_string_configured() {
        // Some("") is a misconfigured token — treated as open (same as None).
        assert!(token_matches(Some(""), ""));
        assert!(token_matches(Some(""), "anything"));
    }

    fn test_app_plex_nowplaying() -> Router {
        // All forward_now_playing flags set to false so fan_out_now_playing
        // does not make real HTTP calls during tests.
        let cfg = Arc::new(Config {
            server: crate::config::ServerConfig { port: 4567, webhook_token: None },
            plex: crate::config::PlexConfig { webhook_token: None },
            jellyfin: crate::config::JellyfinConfig { webhook_token: None },
            koito: crate::config::KoitoConfig {
                base_url: "http://k".into(),
                api_key: "k".into(),
                forward_now_playing: Some(false),
            },
            listenbrainz: crate::config::ListenBrainzConfig {
                user_token: "t".into(),
                forward_now_playing: Some(false),
            },
            lastfm: crate::config::LastFmConfig {
                api_key: "a".into(),
                shared_secret: "s".into(),
                session_key: "k".into(),
                forward_now_playing: Some(false),
            },
        });
        build_router(AppState { cfg, client: reqwest::Client::new() })
    }

    fn plex_nowplaying_request(event_type: &str) -> http::Request<Body> {
        let json = format!(
            r#"{{"event":"{}","Metadata":{{"grandparentTitle":"Radiohead","parentTitle":"OK Computer","title":"Karma Police","duration":264000}}}}"#,
            event_type
        );
        let body = format!(
            "--testboundary\r\nContent-Disposition: form-data; name=\"payload\"\r\n\r\n{}\r\n--testboundary--",
            json
        );
        http::Request::builder()
            .method("POST")
            .uri("/webhooks/plex/open")
            .header("content-type", "multipart/form-data; boundary=testboundary")
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn plex_handler_returns_200_for_media_play() {
        let app = test_app_plex_nowplaying();
        let response = app.oneshot(plex_nowplaying_request("media.play")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn plex_handler_returns_200_for_media_resume() {
        let app = test_app_plex_nowplaying();
        let response = app.oneshot(plex_nowplaying_request("media.resume")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn plex_handler_returns_200_for_unrecognised_event() {
        let app = test_app_plex_nowplaying();
        let response = app.oneshot(plex_nowplaying_request("media.stop")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
