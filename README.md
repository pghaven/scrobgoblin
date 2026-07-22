# Scrobgoblin 👺

[![CI](https://github.com/pghaven/scrobgoblin/actions/workflows/ci.yml/badge.svg)](https://github.com/pghaven/scrobgoblin/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**One webhook listener. Every scrobble target.**

Scrobgoblin is a small, fast Rust service (built on [axum](https://github.com/tokio-rs/axum)) that sits between your self-hosted media servers and your scrobbling services. It exposes a **ListenBrainz-compatible API**, so anything that already knows how to scrobble to ListenBrainz — Navidrome included — can point at Scrobgoblin instead just by changing a URL. It also accepts native Plex and Jellyfin webhooks directly. Every play event is normalized and fanned out — concurrently — to Koito, ListenBrainz, and Last.fm.

## Why

If you run more than one media server, you've probably hit this: each server wants its own scrobbling configuration, and each scrobble target (ListenBrainz, Last.fm, Koito) wants its own plugin or API integration per server. That's a lot of separate integrations to keep track of.

Scrobgoblin collapses that down to one place. Point your media servers at a single service, configure your targets once, and every play event gets normalized and routed automatically:

```
Navidrome (or any    ─┐                  ┌─▶ Koito
LB-compatible client)  │                  │
Plex                  ─┼──▶ Scrobgoblin ──┼─▶ ListenBrainz
Jellyfin              ─┘                  └─▶ Last.fm
```

## Features

- **ListenBrainz-compatible endpoint** — any service that scrobbles to ListenBrainz (Navidrome, and others that speak the same API) can switch to Scrobgoblin by changing its base URL, no plugin needed
- **Native Plex and Jellyfin webhooks** — accepted directly, no LB translation required on their end
- **Modular scrobble targets** — Koito, ListenBrainz, and Last.fm today; each target is an independent, optional implementation of a shared trait, so adding a new one doesn't touch the fan-out or routing logic
- **Concurrent fan-out** — every configured target is scrobbled to independently, in parallel
- **Now-playing support** — forwards "currently playing" updates to targets that support it
- **Retry with backoff** — failed scrobbles retry automatically before being dropped
- **Duration-threshold filtering** — skips scrobbles for tracks that were barely played
- **Per-source webhook auth** — optional token/header checks for Navidrome, Plex, and Jellyfin
- **Docker-first** — one `docker compose up` and it's running alongside the rest of your stack

## Sources → Targets

| Source | How it connects |
| --- | --- |
| Navidrome (or any ListenBrainz-compatible client) | Point it at Scrobgoblin's base URL — it uses the same paths as the real ListenBrainz API (`/1/submit-listens`, `/1/validate-token`) |
| Plex | Webhook URL: `http://<scrobgoblin-host>:<port>/webhooks/plex/<token>` (token required in the URL — see [Security](#security)) |
| Jellyfin | Webhook URL: `http://<scrobgoblin-host>:<port>/webhooks/jellyfin` (requires a webhook template — see [Setup](#setup)) |

| Target | Protocol |
| --- | --- |
| Koito | ListenBrainz-compatible API at `/apis/listenbrainz/1/` |
| ListenBrainz | Native ListenBrainz API |
| Last.fm | `track.scrobble` with MD5 signature |

Each of `[koito]`, `[listenbrainz]`, and `[lastfm]` in `config.toml` is independently optional — omit any section to disable scrobbling to that target. At least one should typically be configured; Scrobgoblin logs a warning at startup if none are, but it isn't enforced.

## Setup

1. Copy `config.toml.example` to `config.toml` and fill in your credentials (see [Configuration](#configuration))
2. Point Navidrome (or any other ListenBrainz-compatible client) at Scrobgoblin's base URL and API key, the same way you would point it at listenbrainz.org
3. Set your Plex webhook URL to `http://<scrobgoblin-host>:<port>/webhooks/plex/<token>` — a token segment is required in the URL even if you leave auth disabled (use any placeholder value, e.g. `open`)
4. For Jellyfin, install the webhook plugin, add the `X-Scroblin-Token` header if you're using auth, and configure a webhook template (Jellyfin sends an empty body without one). Only flat `{{expression}}` placeholders work — Handlebars block helpers (`{{#if}}`, `{{#each}}`) are broken in the bundled plugin version:
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

## Running

```bash
docker compose up -d
```

Or directly:
```bash
cargo run
```

## Configuration

```toml
[server]
port = 4567
# Optional. If set, Navidrome (and any other LB-compatible client) must send
# Authorization: Token <webhook_token>. Omit to allow all requests.
webhook_token = "your-webhook-token"

[plex]
# Optional. Must match the token embedded in the Plex webhook URL.
# webhook_token = "your-plex-secret"

[jellyfin]
# Optional. Must match the X-Scroblin-Token header sent by the webhook plugin.
# webhook_token = "your-jellyfin-secret"

# [koito], [listenbrainz], and [lastfm] are all optional — omit any section
# entirely to disable scrobbling to that target.
[koito]
base_url = "http://koito.yourdomain.com"
api_key  = "your-koito-api-key"
# forward_now_playing = false  # default false — enable only after confirming Koito deduplicates

[listenbrainz]
user_token = "your-listenbrainz-token"
# forward_now_playing = true   # default true

[lastfm]
api_key       = "your-lastfm-api-key"
shared_secret = "your-lastfm-shared-secret"
session_key   = "your-lastfm-session-key"
# forward_now_playing = true   # default true
```

## Scrobble threshold

Tracks under 30 seconds are silently ignored. If a track has no duration in the webhook payload, it is always scrobbled.

## Now playing

Scrobgoblin forwards "now playing" events to targets that support it. Controlled per-target via `forward_now_playing` in `config.toml`:

| Target | Default | API method |
| --- | --- | --- |
| ListenBrainz | `true` | `listen_type: "playing_now"` to `/1/submit-listens` |
| Last.fm | `true` | `track.updateNowPlaying` |
| Koito | `false` | Same LB payload (enable after confirming deduplication) |

Now-playing failures are logged with `[NOW-FAIL]` and not retried.

## Retry behavior

Each target is submitted independently. On failure, Scrobgoblin retries up to 3 times with backoff (1s → 4s). After 3 failures the event is logged and discarded. No persistence across restarts.

## Security

Each source has its own optional token-based auth, disabled by default:

| Source | Mechanism | Config |
| --- | --- | --- |
| Navidrome (LB-compatible) | `Authorization: Token <token>` header | `server.webhook_token` |
| Plex | Token embedded in the webhook URL path | `plex.webhook_token` |
| Jellyfin | `X-Scroblin-Token` header | `jellyfin.webhook_token` |

Leaving a `webhook_token` unset (or blank) allows all requests for that source — reasonable for deployments where the port is only reachable over a trusted internal network (e.g. a Docker Compose bridge network not exposed to the internet). If you expose Scrobgoblin externally, set a token for every source you use. Worst case without auth is spam scrobbles — no upstream credentials are ever exposed through the webhook interface.

## Building

```bash
cargo build --release
docker build -t scrobgoblin:latest .
```

## License

MIT — see [LICENSE](LICENSE).
