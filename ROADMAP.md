# Scroblin Roadmap

---

## ✅ Done — Webhook token authentication

**Status:** Implemented 2026-05-31.

The `validate-token` and `submit-listens` handlers now check the `Authorization: Token <value>` header against `server.webhook_token` in `config.toml`. This matches the token Navidrome already sends via `ND_LISTENBRAINZ_APIKEY`. If the token is unset, all requests are allowed (safe for internal-only deployments). If set, mismatched requests receive HTTP 401.

This closes the public-exposure risk from the Traefik routing at `https://scroblin.geary.quest`.

---

## ✅ Done — Mobile client scrobbling

**Status:** Confirmed working 2026-05-31.

Scrobbling from both the Navidrome web UI and mobile Subsonic clients is confirmed working.

---

## ✅ Done — Plex and Jellyfin webhook authentication

**Status:** Implemented 2026-06-02. Pending production testing.

Plex and Jellyfin webhook handlers now support optional per-source token authentication:
- **Plex:** URL-embedded token — webhook URL becomes `http://scroblin:4567/webhooks/plex/{token}`. Legacy `/webhooks/plex` returns 404 with a helpful migration log.
- **Jellyfin:** Fixed header `X-Scroblin-Token` — configure in Jellyfin's webhook plugin as a custom header.

Both default to open (all requests allowed) when the section is absent from `config.toml`. If set, mismatched requests receive HTTP 401. Token scrubbed from 404 logs to avoid credential exposure.

---

## ✅ Done — Per-target "now playing" forwarding

**Status:** Implemented 2026-05-31.

`playing_now` events from Navidrome are now forwarded to ListenBrainz and Last.fm by default, and optionally to Koito. Per-target `forward_now_playing` flags in `config.toml` control which targets receive these events:
- **ListenBrainz:** default `true` — forwards as `listen_type: "playing_now"` to `/1/submit-listens`
- **Last.fm:** default `true` — calls `track.updateNowPlaying`
- **Koito:** default `false` — enable once Koito deduplication is confirmed

If a flag is omitted from config, defaults apply. Failures are logged with `[NOW-FAIL]` prefix and not retried.

---

## 3 — Structured logging

**Status:** Not started.

Replace `println!`/`eprintln!` with `tracing` crate structured logs. Benefits:
- JSON output mode for log aggregation (Loki/Promtail)
- Log levels (`INFO`, `WARN`, `ERROR`) filterable at runtime via `RUST_LOG`
- Consistent field names (`target`, `artist`, `track`, `attempt`, `listened_at`)
