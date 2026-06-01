# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Git Remote

This project is hosted on Forgejo at forgejo.geary.quest. Use standard `git push` — never `gh` CLI. The `fj` CLI may be used for issues/PRs if needed.

## Common Commands

```bash
cargo build             # Build debug
cargo build --release   # Build release binary
cargo run               # Run locally (requires config.toml in CWD)
cargo test              # Run all tests
cargo test <name>       # Run a single test by name
cargo check             # Fast type-check without building
docker compose up -d    # Run via Docker (requires config.toml mounted)
docker build -t scroblin:latest .
```

## Architecture

Single-binary axum HTTP server. Three webhook endpoints receive play events, normalize them into a canonical `PlayEvent`, apply a duration threshold check, then fan out to three scrobble targets concurrently.

```
Webhook POST → source parser → PlayEvent → threshold::qualifies → fan_out
                                                                      ├── koito::submit
                                                                      ├── listenbrainz::submit
                                                                      └── lastfm::submit
```

**Sources** (`src/sources/`): Each parses its native webhook format into `PlayEvent`. Navidrome sends ListenBrainz format JSON; Plex sends multipart/form-data with a JSON "payload" field; Jellyfin sends JSON. Only `media.scrobble` (Plex) and `PlaybackStopped` (Jellyfin) events qualify — others return 200 silently.

**Targets** (`src/targets/`): Each exposes `submit_to(base_url, credentials, client, event)` for testability with mockito. Koito and ListenBrainz share `build_lb_payload()` from `listenbrainz.rs`. Last.fm uses MD5 signature via `BTreeMap` (alphabetical param ordering guaranteed).

**Threshold** (`src/threshold.rs`): Tracks under 30 seconds are discarded. If duration is absent, the event always qualifies (webhooks fire at completion, not mid-play).

**Fan-out** (`src/targets/mod.rs`): `fan_out` spawns 3 tasks concurrently with retry (1s → 4s backoff, joined). `fan_out_now_playing` is fire-and-forget (tasks spawned, not joined, no retry) — `[NOW-FAIL]` on error.

**Router** (`src/router.rs`): `AppState` holds `Arc<Config>` and a shared `reqwest::Client`. Fan-out is detached via `tokio::spawn` so the webhook handler returns 200 immediately. Navidrome `playing_now` events are parsed via `sources::navidrome::parse_now_playing` and dispatched to `fan_out_now_playing`; always returns `lb_ok()` regardless of parse outcome.

## Key Patterns

- Last.fm scrobble: params have `[0]` suffix (`artist[0]`, `track[0]`, `timestamp[0]`); now-playing uses bare names (`artist`, `track`) — different methods, different param formats
- Last.fm signature: collect params into `BTreeMap`, iterate to build `key=value` string (no separator), append `shared_secret`, MD5 hex encode
- Koito auth: ListenBrainz-compatible `Authorization: Token {api_key}` — not session cookie
- **Koito submit endpoint**: `{base_url}/apis/listenbrainz/1/submit-listens` — NOT `/1/submit-listens`. Koito's LB-compatible API lives under `/apis/listenbrainz/1/`.
- Duration units: Plex sends milliseconds (÷1000), Jellyfin sends `RunTimeTicks` (÷10,000,000), Navidrome sends seconds or `duration_ms`
- Non-qualifying source events (wrong type) return `Err` with string containing "not a scrobble event" or "not a PlaybackStopped event" — router pattern-matches these to return 200
- `NowPlayingEvent` vs `PlayEvent`: same fields minus `played_at`; `build_now_playing_payload` (listenbrainz.rs) is reused by Koito since they share the LB wire format

## Navidrome Gotchas

**`playing_now` events:** Navidrome sends `listen_type: "playing_now"` to `/1/submit-listens` when a track starts, and `listen_type: "single"` when it scrobbles. The handler must filter out `playing_now` events before fan-out — otherwise every song start triggers a duplicate scrobble submission. Koito does not deduplicate; Last.fm and ListenBrainz do. The filter is in `navidrome_handler` in `router.rs`.

**Response body required:** The Navidrome ListenBrainz client checks the response body for `{"status": "ok"}`. Returning a bare HTTP 200 with no body causes Navidrome to log `EOF` and treat the scrobble as failed, queuing it for indefinite retry. The handler must return `Json({"status": "ok"})`, not just `StatusCode::OK`. This was confirmed in production: a backlog of ~2 hours of queued scrobbles was released as soon as the correct response body was added.
