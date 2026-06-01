# Plex and Jellyfin Webhook Authentication Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add optional per-source token authentication to the Plex and Jellyfin webhook handlers so that externally-exposed endpoints can't be abused.

**Architecture:** Two new config structs (`PlexConfig`, `JellyfinConfig`) with `#[serde(default)]` hold an optional token each. A shared `token_matches` helper handles the auth logic. Plex embeds the token in the webhook URL path (`/webhooks/plex/:token`); Jellyfin validates a fixed `X-Scroblin-Token` header. If no token is configured, all requests pass — safe default for internal deployments.

**Tech Stack:** Rust, axum 0.8, serde/toml for config, tower 0.5 (test helper), mockito (existing dev dep).

---

## File Structure

| File | Change |
|------|--------|
| `src/config.rs` | Add `PlexConfig`, `JellyfinConfig` structs; add fields to `Config` |
| `src/router.rs` | Add `token_matches` helper; update `plex_handler` route + signature; update `jellyfin_handler` signature |
| `config.toml.example` | Add commented `[plex]` and `[jellyfin]` sections |
| `Cargo.toml` | Add `tower` and `http` to dev-dependencies for handler tests |

---

### Task 1: Add PlexConfig and JellyfinConfig to config

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Write the failing config tests**

Add to the `#[cfg(test)]` block at the bottom of `src/config.rs`:

```rust
#[test]
fn parses_plex_and_jellyfin_auth_config() {
    let toml = r#"
[server]
port = 4567

[plex]
webhook_token = "plex-secret"

[jellyfin]
webhook_token = "jf-secret"

[koito]
base_url = "http://koito.example.com"
api_key = "koito-key"

[listenbrainz]
user_token = "lb-token"

[lastfm]
api_key = "lfm-key"
shared_secret = "lfm-secret"
session_key = "lfm-session"
"#;
    let cfg: Config = toml::from_str(toml).expect("should parse");
    assert_eq!(cfg.plex.webhook_token.as_deref(), Some("plex-secret"));
    assert_eq!(cfg.jellyfin.webhook_token.as_deref(), Some("jf-secret"));
}

#[test]
fn plex_and_jellyfin_default_to_no_token_when_section_absent() {
    let toml = r#"
[server]
port = 4567

[koito]
base_url = "http://koito.example.com"
api_key = "koito-key"

[listenbrainz]
user_token = "lb-token"

[lastfm]
api_key = "lfm-key"
shared_secret = "lfm-secret"
session_key = "lfm-session"
"#;
    let cfg: Config = toml::from_str(toml).expect("should parse");
    assert!(cfg.plex.webhook_token.is_none());
    assert!(cfg.jellyfin.webhook_token.is_none());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test parses_plex_and_jellyfin_auth_config
cargo test plex_and_jellyfin_default_to_no_token_when_section_absent
```

Expected: FAIL — `Config` has no `plex` or `jellyfin` fields yet.

- [ ] **Step 3: Add the new structs and fields**

In `src/config.rs`, add the two new structs after `ServerConfig` and before `KoitoConfig`:

```rust
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PlexConfig {
    pub webhook_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct JellyfinConfig {
    pub webhook_token: Option<String>,
}
```

Then update the `Config` struct to add the two new fields with `#[serde(default)]`:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(default)]
    pub plex: PlexConfig,
    #[serde(default)]
    pub jellyfin: JellyfinConfig,
    pub koito: KoitoConfig,
    pub listenbrainz: ListenBrainzConfig,
    pub lastfm: LastFmConfig,
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test parses_plex_and_jellyfin_auth_config
cargo test plex_and_jellyfin_default_to_no_token_when_section_absent
cargo test
```

Expected: all 38 tests pass (36 existing + 2 new).

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat: add PlexConfig and JellyfinConfig with optional webhook_token"
```

---

### Task 2: Add token_matches helper

**Files:**
- Modify: `src/router.rs`

- [ ] **Step 1: Write the failing tests**

Add a `#[cfg(test)]` block at the bottom of `src/router.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test token_matches
```

Expected: FAIL — `token_matches` is not defined yet.

- [ ] **Step 3: Add the token_matches function**

Add this function to `src/router.rs` alongside the existing `authorized` function (after line 70, before `fn lb_ok()`):

```rust
fn token_matches(expected: Option<&str>, provided: &str) -> bool {
    match expected {
        None => true,
        Some(t) => t == provided,
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test token_matches
cargo test
```

Expected: all 41 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/router.rs
git commit -m "feat: add token_matches helper for per-source webhook auth"
```

---

### Task 3: Wire Plex handler auth

**Files:**
- Modify: `src/router.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Add test dev-dependencies**

In `Cargo.toml`, add to `[dev-dependencies]`:

```toml
tower = { version = "0.5", features = ["util"] }
http = "1"
```

- [ ] **Step 2: Write the failing Plex handler tests**

Add these tests to the `tests` module in `src/router.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use axum::body::Body;
    use tower::ServiceExt;

    // ... existing token_matches tests ...

    fn test_app_plex_token(plex_token: Option<&str>) -> Router {
        let cfg = Arc::new(Config {
            server: ServerConfig { port: 4567, webhook_token: None },
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
}
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
cargo test plex_handler_rejects_wrong_url_token
cargo test plex_handler_allows_when_no_token_configured
```

Expected: FAIL — route is still `/webhooks/plex` (no `:token` segment) and handler has no auth check.

- [ ] **Step 4: Update the Plex route and handler**

In `build_router` in `src/router.rs`, change the Plex route:

```rust
// Before:
.route("/webhooks/plex", post(plex_handler))

// After:
.route("/webhooks/plex/:token", post(plex_handler))
```

Update the `plex_handler` signature to extract the path token and check auth. The full updated handler:

```rust
async fn plex_handler(
    State(state): State<AppState>,
    axum::extract::Path(url_token): axum::extract::Path<String>,
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
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test plex_handler_rejects_wrong_url_token
cargo test plex_handler_allows_when_no_token_configured
cargo test
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/router.rs Cargo.toml
git commit -m "feat: add URL-token auth to Plex webhook handler"
```

---

### Task 4: Wire Jellyfin handler auth

**Files:**
- Modify: `src/router.rs`

- [ ] **Step 1: Write the failing Jellyfin handler tests**

Add these tests to the `tests` module in `src/router.rs` (alongside the Plex tests):

```rust
fn test_app_jellyfin_token(jellyfin_token: Option<&str>) -> Router {
    let cfg = Arc::new(Config {
        server: ServerConfig { port: 4567, webhook_token: None },
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
                .body(Body::from("{}"))
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
                .body(Body::from("{}"))
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
    // JSON body is invalid so we expect 400, but NOT 401.
    let response = app
        .oneshot(
            http::Request::builder()
                .method("POST")
                .uri("/webhooks/jellyfin")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test jellyfin_handler_rejects_wrong_header_token
cargo test jellyfin_handler_rejects_missing_header_when_token_configured
cargo test jellyfin_handler_allows_when_no_token_configured
```

Expected: FAIL — `jellyfin_handler` has no auth check yet.

- [ ] **Step 3: Update the Jellyfin handler**

Update `jellyfin_handler` in `src/router.rs` to extract `HeaderMap` and validate `X-Scroblin-Token`. The full updated handler:

```rust
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
```

Note: axum deserializes the `Json` body before the handler runs. A missing or invalid JSON body will return 422 from axum before auth is checked. This is acceptable — the auth check happens first for valid JSON requests, and malformed payloads are rejected regardless.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test jellyfin_handler_rejects_wrong_header_token
cargo test jellyfin_handler_rejects_missing_header_when_token_configured
cargo test jellyfin_handler_allows_when_no_token_configured
cargo test
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/router.rs
git commit -m "feat: add X-Scroblin-Token header auth to Jellyfin webhook handler"
```

---

### Task 5: Update config.toml.example and push

**Files:**
- Modify: `config.toml.example`

- [ ] **Step 1: Add the new sections to config.toml.example**

Open `config.toml.example`. After the `[server]` block and before `[koito]`, add:

```toml
[plex]
# Embed this token in your Plex webhook URL:
#   http://scroblin:4567/webhooks/plex/your-plex-secret
# The token segment is required even with no auth — use any value (e.g., "open").
# webhook_token = "your-plex-secret"

[jellyfin]
# In Jellyfin's webhook plugin, add header: X-Scroblin-Token: your-jellyfin-secret
# webhook_token = "your-jellyfin-secret"
```

- [ ] **Step 2: Run the full test suite one final time**

```bash
cargo test
```

Expected: all tests pass (no regressions).

- [ ] **Step 3: Commit and push**

```bash
git add config.toml.example
git commit -m "docs: add plex and jellyfin auth config examples"
git push http://paul:<token>@forgejo.geary.quest/paul/scroblin.git HEAD
```

Replace `<token>` with your Forgejo token from `~/.claude/cycle-close.local.md`.
