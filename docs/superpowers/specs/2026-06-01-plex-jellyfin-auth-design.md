# Plex and Jellyfin Webhook Authentication — Design Spec

## Goal

Add optional per-source authentication to the Plex and Jellyfin webhook handlers. If a token is configured, requests that don't carry the correct secret are rejected with HTTP 401. If no token is configured, all requests are allowed — preserving the safe default for internal-only deployments.

## Background

Scroblin is externally reachable at `https://scroblin.geary.quest` via Traefik. The Navidrome endpoint already requires `Authorization: Token <value>` via `server.webhook_token`. The Plex and Jellyfin handlers are currently unauthenticated, exposing them to spam scrobbles from any caller.

---

## Config Changes (`src/config.rs`)

Add two new optional top-level config sections:

```toml
[plex]
webhook_token = "your-plex-secret"   # omit section or field to allow all requests

[jellyfin]
webhook_token = "your-jellyfin-secret"  # omit section or field to allow all requests
```

New structs:

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

`Config` gains two new fields with serde defaults:

```rust
pub struct Config {
    pub server: ServerConfig,
    #[serde(default)]
    pub plex: PlexConfig,
    #[serde(default)]
    pub jellyfin: JellyfinConfig,
    // ... existing target fields
}
```

Using `#[serde(default)]` (rather than `Option<PlexConfig>`) means the section being absent is equivalent to the section being present with no `webhook_token` — both disable auth. This is simpler to use in the handler (`cfg.plex.webhook_token` rather than `cfg.plex.as_ref().and_then(...)`).

---

## Auth Helper (`src/router.rs`)

Extract a standalone helper used by all three source handlers:

```rust
fn token_matches(expected: Option<&str>, provided: &str) -> bool {
    match expected {
        None => true,           // auth disabled
        Some(t) => t == provided,
    }
}
```

The existing `authorized()` function (which reads from `Authorization: Token` headers for Navidrome) is kept as-is. The new `token_matches` handles the simpler cases where the token is already extracted from a path param or header value.

---

## Plex Handler

**Mechanism:** URL-embedded token. Plex webhooks don't support custom headers; embedding the secret in the URL is the standard approach.

**Route change:**
```
/webhooks/plex  →  /webhooks/plex/:token
```

**Webhook URL to configure in Plex:** `https://scroblin.geary.quest/webhooks/plex/your-secret`

**Handler change:** Extract `:token` path parameter. Call `token_matches(cfg.plex.webhook_token.as_deref(), &token)`. Return `StatusCode::UNAUTHORIZED` on mismatch. Log `[WARN] Plex auth failed` (no token value in log).

**Safe default:** If `plex.webhook_token` is `None`, `token_matches` returns `true` regardless of the URL token. Existing webhook URL `/webhooks/plex` will no longer match the route — users who currently send to that URL must update it to include a token segment (even a placeholder like `/webhooks/plex/open` if they don't want auth).

> **Note:** Because the route now requires a `:token` segment, users must update their Plex webhook URL even if they don't want auth — they can use any value (e.g., `/webhooks/plex/public`). This is documented in `config.toml.example`.

---

## Jellyfin Handler

**Mechanism:** Fixed custom header `X-Scroblin-Token`. Jellyfin's webhook plugin supports configuring arbitrary request headers.

**Route:** Unchanged — `/webhooks/jellyfin`.

**Handler change:** Add `HeaderMap` to handler parameters. Extract `X-Scroblin-Token` header value. Call `token_matches(cfg.jellyfin.webhook_token.as_deref(), provided)`. Return `StatusCode::UNAUTHORIZED` on mismatch. Log `[WARN] Jellyfin auth failed`.

**Safe default:** If `jellyfin.webhook_token` is `None`, all requests pass regardless of whether the header is present.

**Webhook config in Jellyfin:** In the webhook plugin, add header `X-Scroblin-Token` with value matching `jellyfin.webhook_token`.

---

## `config.toml.example` Updates

```toml
[plex]
# webhook_token = "your-plex-secret"
# Webhook URL: http://scroblin:4567/webhooks/plex/your-plex-secret
# (token is part of the URL — required segment even if auth is disabled, use any value)

[jellyfin]
# webhook_token = "your-jellyfin-secret"
# In Jellyfin webhook plugin, add header: X-Scroblin-Token: your-jellyfin-secret
```

---

## Error Handling

| Scenario | Behaviour |
|----------|-----------|
| `plex.webhook_token` absent | All Plex requests allowed |
| `plex.webhook_token` set, URL token missing or wrong | HTTP 401, log `[WARN] Plex auth failed` |
| `plex.webhook_token` set, URL token correct | Normal processing |
| `jellyfin.webhook_token` absent | All Jellyfin requests allowed |
| `jellyfin.webhook_token` set, header absent or wrong | HTTP 401, log `[WARN] Jellyfin auth failed` |
| `jellyfin.webhook_token` set, header correct | Normal processing |

Token values are never logged.

---

## Testing

**`src/config.rs`:**
- Section present with `webhook_token` → parses correctly
- Section present without `webhook_token` → `webhook_token` is `None`
- Section absent entirely → `PlexConfig`/`JellyfinConfig` default, `webhook_token` is `None`

**`src/router.rs` — Plex handler:**
- Token configured, correct URL token → 200
- Token configured, wrong URL token → 401
- No token configured → 200 regardless of URL token

**`src/router.rs` — Jellyfin handler:**
- Token configured, correct `X-Scroblin-Token` header → 200
- Token configured, wrong header value → 401
- Token configured, header absent → 401
- No token configured → 200 regardless of header
