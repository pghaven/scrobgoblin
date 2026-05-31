# Scroblin Design Spec

**Date:** 2026-05-31  
**Status:** Approved

## Overview

Scroblin is a small, efficient Rust service that receives play events from three media servers (Navidrome, Plex, Jellyfin) via webhooks and fans them out to three scrobble targets (Koito, ListenBrainz, Last.fm) simultaneously. It replaces multi-scrobbler with a purpose-built, zero-polling, minimal-dependency alternative.

**Goals:** low RAM (~5–10 MB idle), zero polling, no database, Docker-deployable.

---

## Architecture

Single `scroblin` binary running an axum HTTP server. Webhook handlers normalize payloads into a canonical `PlayEvent`, apply the scrobble threshold, then fan out concurrently to all three targets via independent tokio tasks with retry.

**Data flow:**
```
Webhook → handler → parse → PlayEvent → threshold check → fan-out (3 tokio tasks) → each target
```

**Project layout:**
```
Scroblin/
├── Cargo.toml
├── config.toml          # credentials + URLs (gitignored)
├── config.toml.example
├── Dockerfile
├── src/
│   ├── main.rs          # startup, config load, router setup
│   ├── config.rs        # Config struct, TOML deserialization
│   ├── event.rs         # PlayEvent — canonical internal type
│   ├── threshold.rs     # 50%/4-min scrobble rule
│   ├── router.rs        # axum routes + webhook handlers
│   ├── sources/
│   │   ├── navidrome.rs # ListenBrainz webhook payload → PlayEvent
│   │   ├── plex.rs      # Plex webhook JSON → PlayEvent
│   │   └── jellyfin.rs  # Jellyfin webhook JSON → PlayEvent
│   └── targets/
│       ├── mod.rs       # fan-out + retry logic
│       ├── koito.rs     # POST to Koito LB-compatible endpoint
│       ├── listenbrainz.rs
│       └── lastfm.rs    # MD5 auth signature + track.scrobble
└── tests/
    └── ...              # unit tests per source/target
```

---

## Core Types

### PlayEvent

The single internal representation all sources normalize into:

```rust
pub struct PlayEvent {
    pub artist: String,
    pub album: Option<String>,
    pub track: String,
    pub duration_secs: Option<u64>,
    pub played_at: DateTime<Utc>,
    pub source: Source,
}

pub enum Source { Navidrome, Plex, Jellyfin }
```

### Threshold Logic

Since webhooks fire at play completion (not mid-track), elapsed time is not available — the webhook itself signals that the track was played. The threshold check is therefore: scrobble if `duration_secs >= 30` (discard very short tracks/interludes). If `duration_secs` is absent, scrobble unconditionally — better to over-report than silently drop. The 50%/4-min elapsed rule cannot be enforced without a "now playing" → "stopped" event pair; trusting completion webhooks is the correct tradeoff here.

---

## Sources

### Navidrome
Sends the ListenBrainz `listen` format to the configured base URL. Event arrives at `POST /webhooks/navidrome`. Relevant fields: `track_metadata.artist_name`, `track_metadata.track_name`, `track_metadata.release_name`, `track_metadata.additional_info.duration`.

### Plex
Sends multipart form with a JSON `payload` field. Filter to `media.scrobble` event type only. Relevant fields: `Metadata.grandparentTitle` (artist), `Metadata.parentTitle` (album), `Metadata.title` (track), `Metadata.duration` (milliseconds — divide by 1000).

### Jellyfin
Sends JSON via the webhook plugin. Filter to `PlaybackStopped` event. Relevant fields: `Artist`, `Album`, `Name`, `RunTimeTicks` (100ns units — divide by 10,000,000).

---

## Targets

### Koito
- Endpoint: `POST <base_url>/1/submit-listens`
- Auth: `Authorization: Token <api_key>`
- Body: standard ListenBrainz JSON (`listen_type: "single"`, `listened_at`, `track_metadata`)

### ListenBrainz
- Endpoint: `POST https://api.listenbrainz.org/1/submit-listens`
- Auth: `Authorization: Token <user_token>`
- Body: same LB format as Koito — shares payload builder, differs only in base URL and token

### Last.fm
- Endpoint: `POST https://ws.audioscrobbler.com/2.0/`
- Method: `track.scrobble`
- Auth: MD5 signature — sort all params alphabetically, concatenate as `key=value` (no separator), append shared secret, MD5 hash the result
- Required params: `method`, `api_key`, `sk` (session key), `artist`, `track`, `timestamp` (unix)
- Optional params: `album`, `duration`

---

## Retry Behavior

Each target task retries independently:
- Attempt 1: immediate
- Attempt 2: wait 1s
- Attempt 3: wait 4s
- After 3 failures: log error with source/artist/track and discard

No persistence of failed scrobbles across restarts.

---

## Configuration

**`config.toml`:**
```toml
[server]
port = 4567

[koito]
base_url  = "http://koito.example.com"
api_key   = "..."

[listenbrainz]
user_token = "..."

[lastfm]
api_key       = "..."
shared_secret = "..."
session_key   = "..."
```

---

## Deployment

**Docker image:** Multi-stage build — compile in `rust:slim`, copy binary into `debian:bookworm-slim`. Target image ~20–30 MB, idle RAM ~5–10 MB.

**docker-compose.yml:**
```yaml
services:
  scroblin:
    image: scroblin:latest
    ports:
      - "4567:4567"
    volumes:
      - ./config.toml:/app/config.toml:ro
    restart: unless-stopped
```

**Webhook URLs:**
- Navidrome → ListenBrainz Base URL: `http://scroblin:4567/webhooks/navidrome`
- Plex → Webhooks: `http://scroblin:4567/webhooks/plex`
- Jellyfin → Webhook plugin: `http://scroblin:4567/webhooks/jellyfin`

---

## Dependencies

```toml
axum = "0.8"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "time"] }
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
chrono = { version = "0.4", features = ["serde"] }
md5 = "0.7"
```

---

## Logging

Structured plaintext to stdout (captured by Docker). Format:
- Success: `[OK] Navidrome → LastFm | Artist - Track`
- Failure: `[FAIL] Navidrome → LastFm | Artist - Track | attempt 2/3 | <error>`
