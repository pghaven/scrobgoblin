# Scrobgoblin

A small, efficient Rust service that receives play webhooks from Navidrome, Plex, and Jellyfin and fans them out to Koito, ListenBrainz, and Last.fm simultaneously.

## Sources → Targets

| Source | Configuration |
|--------|--------------|
| Navidrome | Set `ND_LISTENBRAINZ_BASEURL=http://scrobgoblin:4567` — Navidrome uses the ListenBrainz API paths (`/1/submit-listens`, `/1/validate-token`) |
| Plex | Webhook URL: `http://scrobgoblin:4567/webhooks/plex` |
| Jellyfin | Webhook URL: `http://scrobgoblin:4567/webhooks/jellyfin` |

| Target | Protocol |
|--------|----------|
| Koito | ListenBrainz-compatible API at `/apis/listenbrainz/1/` |
| ListenBrainz | ListenBrainz API |
| Last.fm | track.scrobble with MD5 signature |

Each of `[koito]`, `[listenbrainz]`, and `[lastfm]` in `config.toml` is optional — omit any section to disable scrobbling to that target. At least one should typically be configured, but this isn't enforced.

## Setup

1. Copy `config.toml.example` to `config.toml` and fill in your credentials
2. Configure each media server to send webhooks to the URLs above
3. For Jellyfin, configure the webhook plugin to send `PlaybackStopped` events

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

[koito]
base_url = "http://koito.yourdomain.com"
api_key  = "your-koito-api-key"

[listenbrainz]
user_token = "your-listenbrainz-token"

[lastfm]
api_key       = "your-lastfm-api-key"
shared_secret = "your-lastfm-shared-secret"
session_key   = "your-lastfm-session-key"
```

## Scrobble threshold

Tracks under 30 seconds are silently ignored. If a track has no duration in the webhook payload, it is always scrobbled.

## Navidrome notes

Navidrome sends two event types to `/1/submit-listens`:
- `listen_type: "playing_now"` — fired when a track starts. Forwarded to configured targets as a "now playing" notification (see Now Playing below).
- `listen_type: "single"` — fired when the track scrobbles (past the threshold). Fanned out to all targets.

Navidrome requires a `{"status": "ok"}` JSON body in the response. An empty HTTP 200 is treated as a failure and the scrobble is queued for indefinite retry.

## Now playing

Scrobgoblin forwards `playing_now` events to targets that support it. Controlled per-target via `forward_now_playing` in `config.toml`:

| Target | Default | API method |
|--------|---------|------------|
| ListenBrainz | `true` | `listen_type: "playing_now"` to `/1/submit-listens` |
| Last.fm | `true` | `track.updateNowPlaying` |
| Koito | `false` | Same LB payload (enable after confirming deduplication) |

Now-playing failures are logged with `[NOW-FAIL]` and not retried.

## Retry behavior

Each target is submitted independently. On failure, Scrobgoblin retries up to 3 times with backoff (1s → 4s). After 3 failures the event is logged and discarded. No persistence across restarts.

## Security note

The webhook listener binds on `0.0.0.0` with no authentication. In a typical Docker Compose deployment on a home network, all three media servers communicate over Docker's internal bridge network and the port is not exposed to the internet — this is safe.

If you ever expose port 4567 externally (e.g., via a reverse proxy or port forwarding), consider adding:
- **Plex**: verify the `X-Plex-Token` header against a token in config
- **Jellyfin**: configure a webhook secret and verify it against a header
- **Navidrome**: IP allowlist or reverse-proxy basic auth (the LB endpoint has no built-in auth)

The worst-case impact without auth is spam scrobbles — credentials are never exposed through the webhook interface.

## Building

```bash
cargo build --release
docker build -t scrobgoblin:latest .
```
