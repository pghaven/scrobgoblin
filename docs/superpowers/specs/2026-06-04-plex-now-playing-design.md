# Plex Now-Playing Design

**Date:** 2026-06-04

## Goal

When Plex fires a `media.play` or `media.resume` webhook event, fan out a now-playing update to all configured scrobble targets — the same path already used for Navidrome now-playing.

## Context

Plex sends all webhook events to a single URL. The `plex_handler` already handles `media.scrobble`. All now-playing infrastructure (`NowPlayingEvent`, `fan_out_now_playing`, `Source::Plex`) exists and is used by Navidrome; this feature wires Plex into it.

The per-target `forward_now_playing` flags (`[koito]`, `[listenbrainz]`, `[lastfm]`) already gate forwarding — no new config is needed.

## Changes

### `src/sources/plex.rs`

Add `parse_now_playing(payload: &PlexPayload) -> Result<NowPlayingEvent>`.

- Returns `Err` if `payload.event` is not `"media.play"` or `"media.resume"` (mirrors how `parse` returns `Err` for non-scrobble events)
- Constructs `NowPlayingEvent` from `payload.metadata` (same fields as `parse`: `grandparentTitle` → artist, `parentTitle` → album, `title` → track, `duration` ms → secs)
- `source: Source::Plex`

### `src/router.rs` — `plex_handler`

After `PlexPayload` is parsed from the multipart payload, match on `payload.event`:

```
"media.play" | "media.resume" → parse_now_playing → spawn fan_out_now_playing → return 200
"media.scrobble"               → existing scrobble path (unchanged)
_                              → silent 200 (unchanged)
```

The multipart parsing, token auth, and error handling remain identical. Only the dispatch branch is new.

Log line: `[REQ] playing_now (plex) | {artist} - {track}` (consistent with Navidrome's `playing_now` log).

### Tests

**`src/sources/plex.rs`:**
- `parse_now_playing` returns `Ok` for `media.play`
- `parse_now_playing` returns `Ok` for `media.resume`
- `parse_now_playing` returns `Err` for non-play events (e.g. `media.stop`)

**`src/router.rs`:**
- `media.play` webhook returns 200 and dispatches now-playing (use `forward_now_playing: Some(false)` on all targets to avoid real HTTP in tests, same pattern as existing now-playing tests)
- `media.resume` webhook returns 200 and dispatches now-playing
- `media.stop` webhook returns 200 silently (no dispatch)
