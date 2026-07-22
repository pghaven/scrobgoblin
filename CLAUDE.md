# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Config

`config.toml` lives in `conf/config.toml`. A symlink at the repo root (`config.toml → conf/config.toml`) keeps `cargo run` working from the project root.

The Docker compose mounts the `conf/` **directory** (not the file) and runs the binary from it:

```yaml
volumes:
  - ./conf:/conf:ro
command: ["/bin/sh", "-c", "cd /conf && exec /app/scrobgoblin"]
```

**Why a directory mount?** Colima's sshfs layer can't reliably bind-mount individual files — Docker fails with `mkdir /Users/paul: file exists` even when the file exists. Mounting the containing directory avoids this. The app reads `config.toml` from its CWD, so changing into `/conf` before exec is the only other required change.

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
docker compose up -d    # Run via Docker (mounts conf/ directory; see Config above)
docker build -t scrobgoblin:latest .
```

## Architecture

Single-binary axum HTTP server. Three webhook endpoints receive play events, normalize them into canonical event types, apply a duration threshold check (scrobbles only), then fan out to three scrobble targets concurrently.

**Scrobble path:**
```
Webhook POST → source parser → PlayEvent → threshold::qualifies → fan_out
                                                                     ├── koito::submit
                                                                     ├── listenbrainz::submit
                                                                     └── lastfm::submit
```

**Now-playing path:**
```
Webhook POST → source parser → NowPlayingEvent → fan_out_now_playing
                                                      ├── koito::submit_now_playing   (if forward_now_playing = true)
                                                      ├── listenbrainz::submit_now_playing
                                                      └── lastfm::update_now_playing
```

**Sources** (`src/sources/`): Each parses its native webhook format. Navidrome sends ListenBrainz format JSON; Plex sends multipart/form-data with a JSON `payload` field; Jellyfin sends JSON. Event dispatch by source:

| Source | Scrobble trigger | Now-playing trigger |
|--------|-----------------|---------------------|
| Navidrome | `listen_type: "single"` | `listen_type: "playing_now"` |
| Plex | `media.scrobble` | `media.play`, `media.resume` |
| Jellyfin | `PlaybackStop` (position > 0) | `PlaybackStart` |

**Targets** (`src/targets/`): Each target implements the `ScrobbleTarget` trait (`name`, `submit`, `submit_now_playing` with a no-op default, `submit_with_retry` with a shared default retry policy) — see `src/targets/mod.rs`. `KoitoTarget`, `ListenBrainzTarget`, `LastFmTarget` each own their config + a cloned `reqwest::Client`, constructed via `from_config`. `build_targets(&Config, Client) -> Vec<Arc<dyn ScrobbleTarget>>` constructs only the targets whose `config.toml` section is present — each of `[koito]`, `[listenbrainz]`, `[lastfm]` is independently optional. Each still exposes `submit_to(base_url, credentials, client, event)` free functions for testability with mockito. Koito and ListenBrainz share `build_lb_payload()` from `listenbrainz.rs`. Last.fm uses MD5 signature via `BTreeMap` (alphabetical param ordering guaranteed).

**Threshold** (`src/threshold.rs`): Tracks under 30 seconds are discarded. If duration is absent, the event always qualifies (webhooks fire at completion, not mid-play).

**Fan-out** (`src/targets/mod.rs`): `fan_out`/`fan_out_now_playing` are thin wrappers around a shared `dispatch()` helper that spawns one task per target in `AppState.targets`. `fan_out` joins all tasks (retry via each target's `submit_with_retry` default, 1s → 4s backoff). `fan_out_now_playing` does not join (fire-and-forget, no retry) — `[NOW-FAIL]` on error. Adding a new target requires only a new struct + `ScrobbleTarget` impl + one line in `build_targets()` — no changes to `dispatch`, `fan_out`, or `fan_out_now_playing`. Koito now-playing defaults to off (`forward_now_playing = false`); ListenBrainz and Last.fm default to on.

**Router** (`src/router.rs`): `AppState` holds `Arc<Config>` and a shared `reqwest::Client`. Fan-out is detached via `tokio::spawn` so the webhook handler returns 200 immediately. All three source handlers use an explicit `match` on the event type string, mirroring the same pattern: play/resume/start → now-playing, scrobble/stop → scrobble, anything else → 200 silently. `token_matches(expected: Option<&str>, provided: &str) -> bool` handles per-source auth; `None` and `Some("")` both treat as open.

**Startup logging** (`src/main.rs`): after `build_targets`, logs `Active scrobble targets: <names>` or a `[WARN]` if the list is empty — useful for confirming `config.toml` section names (`[koito]`/`[listenbrainz]`/`[lastfm]`) loaded correctly.

## Key Patterns

- Last.fm scrobble: params have `[0]` suffix (`artist[0]`, `track[0]`, `timestamp[0]`); now-playing uses bare names (`artist`, `track`) — different methods, different param formats
- Last.fm signature: collect params into `BTreeMap`, iterate to build `key=value` string (no separator), append `shared_secret`, MD5 hex encode
- Koito auth: ListenBrainz-compatible `Authorization: Token {api_key}` — not session cookie
- **Koito submit endpoint**: `{base_url}/apis/listenbrainz/1/submit-listens` — NOT `/1/submit-listens`. Koito's LB-compatible API lives under `/apis/listenbrainz/1/`.
- Duration units: Plex sends milliseconds (÷1000), Jellyfin sends `RunTimeTicks` in 100-nanosecond units (÷10,000,000), Navidrome sends seconds or `duration_ms`
- Plex webhook auth: URL-embedded token, route `/webhooks/plex/{token}`. Legacy `/webhooks/plex` returns 404 with migration hint. Token scrubbed from 404 logs.
- Jellyfin webhook auth: `X-Scroblin-Token` header. Fixed header name — not configurable.
- LB `playing_now` payload: `additional_info` must be nested inside `track_metadata`, not at the listen level
- `NowPlayingEvent` vs `PlayEvent`: same fields minus `played_at`; `build_now_playing_payload` (listenbrainz.rs) is reused by Koito since they share the LB wire format
- Navidrome/ListenBrainz auth: separate from Plex/Jellyfin's `token_matches()` — uses `authorized()` in router.rs, checking `Authorization: Token <server.webhook_token>` header against `ServerConfig.webhook_token`. Applies to `/1/submit-listens`, `/submit-listens`, and `/validate-token`.

## Navidrome Gotchas

**`playing_now` events:** Navidrome sends `listen_type: "playing_now"` to `/1/submit-listens` when a track starts, and `listen_type: "single"` when it scrobbles. The handler must filter out `playing_now` events before fan-out — otherwise every song start triggers a duplicate scrobble submission. Koito does not deduplicate; Last.fm and ListenBrainz do. The filter is in `navidrome_handler` in `router.rs`.

**Response body required:** The Navidrome ListenBrainz client checks the response body for `{"status": "ok"}`. Returning a bare HTTP 200 with no body causes Navidrome to log `EOF` and treat the scrobble as failed, queuing it for indefinite retry. The handler must return `Json({"status": "ok"})`, not just `StatusCode::OK`. This was confirmed in production: a backlog of ~2 hours of queued scrobbles was released as soon as the correct response body was added.

## Plex Gotchas

**`titleSort` fallback:** Plex occasionally stores the display title blank with the real name in `titleSort` (observed with a‐ha "Take On Me"). `PlexMetadata::effective_title()` returns `titleSort` when `title` is empty. Both `parse()` and `parse_now_playing()` use `effective_title()`.

**Now-playing events:** `media.play` and `media.resume` both fire now-playing updates. `media.play` fires when playback starts from the beginning; `media.resume` fires when resuming a paused track. Both dispatch to `fan_out_now_playing`.

**Webhook format:** Plex sends `multipart/form-data` with a single field named `payload` containing JSON. Unlike Jellyfin and Navidrome, this is not a plain JSON body — axum's `Json` extractor will not work; use the `Multipart` extractor.

## Jellyfin Gotchas

**Event type is `"PlaybackStop"` not `"PlaybackStopped"`:** Jellyfin's webhook plugin sends the C# enum name `PlaybackStop` in the `NotificationType` field. The session manager logs say "Playback stopped" in English but that has no bearing on the wire value. Writing `"PlaybackStopped"` in the parser silently swallows every event.

**Duplicate `PlaybackStop` events:** Jellyfin fires two `PlaybackStop` notifications per track — one at the real playback position and one at position 0 (session cleanup when the next track starts). The position-0 event is filtered in `parse()` via `played_position_ticks == Some(0)`. Without this filter every song scrobbles twice.

**Webhook template is required:** The Jellyfin Generic Client webhook sends an empty body unless a Handlebars template is configured. An empty body causes Scrobgoblin to return 400 (`EOF while parsing JSON`). The template is stored base64-encoded in `Jellyfin/config/plugins/configurations/Jellyfin.Plugin.Webhook.xml`. Edit the XML directly rather than through the Jellyfin UI to avoid encoding issues.

Current template (decoded):
```json
{
    "NotificationType": "{{NotificationType}}",
    "Artist": "{{Artist}}",
    "Album": "{{Album}}",
    "Name": "{{Name}}",
    "RunTimeTicks": {{RunTimeTicks}},
    "PlaybackPositionTicks": {{PlaybackPositionTicks}}
}
```

**Handlebars.NET block helpers are broken:** `{{#if ...}}...{{else}}...{{/if}}` throws `HandlebarsCompilerException: Starting and ending handlebars do not match` in the version bundled with the webhook plugin. Use plain `{{expression}}` only. Do not use `{{#if}}`, `{{#each}}`, or `{{#with}}`. For optional numeric fields, rely on the value always being present for music events rather than guarding with a conditional.

**`RunTimeTicks` units:** 100-nanosecond ticks. Divide by 10,000,000 to get seconds. A 4-minute track is `2,400,000,000` ticks = 240 seconds.

**Koito does not support now-playing:** `forward_now_playing` defaults to `false` for Koito. Leaving it unset in `config.toml` is correct. Last.fm and ListenBrainz default to `true`.
