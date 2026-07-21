pub mod koito;
pub mod lastfm;
pub mod listenbrainz;

use crate::{config::Config, event::{NowPlayingEvent, PlayEvent}};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[async_trait]
pub trait ScrobbleTarget: Send + Sync {
    fn name(&self) -> &'static str;

    async fn submit(&self, event: &PlayEvent) -> anyhow::Result<()>;

    /// Default no-op: targets that don't support now-playing write zero code.
    async fn submit_now_playing(&self, _event: &NowPlayingEvent) -> anyhow::Result<()> {
        Ok(())
    }

    /// Default retry policy: 3 attempts, 1s -> 4s backoff. Individual targets
    /// may override if a specific API needs different retry behavior.
    async fn submit_with_retry(&self, event: &PlayEvent) {
        for attempt in 1..=3u32 {
            match self.submit(event).await {
                Ok(()) => {
                    println!(
                        "[OK] {} → {} | {} - {}",
                        event.source, self.name(), event.artist, event.track
                    );
                    return;
                }
                Err(e) => retry_log(self.name(), event, attempt, &e).await,
            }
        }
    }
}

pub async fn fan_out(cfg: Arc<Config>, client: reqwest::Client, event: PlayEvent) {
    let event = Arc::new(event);

    let (cfg1, client1, event1) = (cfg.clone(), client.clone(), event.clone());
    let t1 = tokio::spawn(async move {
        for attempt in 1..=3u32 {
            match koito::submit(&cfg1.koito, &client1, &event1).await {
                Ok(()) => {
                    println!("[OK] {} → Koito | {} - {}", event1.source, event1.artist, event1.track);
                    break;
                }
                Err(e) => retry_log("Koito", &event1, attempt, &e).await,
            }
        }
    });

    let (cfg2, client2, event2) = (cfg.clone(), client.clone(), event.clone());
    let t2 = tokio::spawn(async move {
        for attempt in 1..=3u32 {
            match listenbrainz::submit(&cfg2.listenbrainz, &client2, &event2).await {
                Ok(()) => {
                    println!("[OK] {} → ListenBrainz | {} - {}", event2.source, event2.artist, event2.track);
                    break;
                }
                Err(e) => retry_log("ListenBrainz", &event2, attempt, &e).await,
            }
        }
    });

    let (cfg3, client3, event3) = (cfg.clone(), client.clone(), event.clone());
    let t3 = tokio::spawn(async move {
        for attempt in 1..=3u32 {
            match lastfm::submit(&cfg3.lastfm, &client3, &event3).await {
                Ok(()) => {
                    println!("[OK] {} → Last.fm | {} - {}", event3.source, event3.artist, event3.track);
                    break;
                }
                Err(e) => retry_log("Last.fm", &event3, attempt, &e).await,
            }
        }
    });

    let _ = tokio::join!(t1, t2, t3);
}

pub async fn fan_out_now_playing(cfg: Arc<Config>, client: reqwest::Client, event: NowPlayingEvent) {
    let event = Arc::new(event);

    if cfg.listenbrainz.forward_now_playing.unwrap_or(true) {
        let (cfg1, client1, event1) = (cfg.clone(), client.clone(), event.clone());
        tokio::spawn(async move {
            match listenbrainz::submit_now_playing(&cfg1.listenbrainz, &client1, &event1).await {
                Ok(()) => println!("[NOW] {} → ListenBrainz | {} - {}", event1.source, event1.artist, event1.track),
                Err(e) => eprintln!("[NOW-FAIL] {} → ListenBrainz now-playing | {} - {} | {}", event1.source, event1.artist, event1.track, e),
            }
        });
    }

    if cfg.lastfm.forward_now_playing.unwrap_or(true) {
        let (cfg2, client2, event2) = (cfg.clone(), client.clone(), event.clone());
        tokio::spawn(async move {
            match lastfm::update_now_playing(&cfg2.lastfm, &client2, &event2).await {
                Ok(()) => println!("[NOW] {} → Last.fm | {} - {}", event2.source, event2.artist, event2.track),
                Err(e) => eprintln!("[NOW-FAIL] {} → Last.fm now-playing | {} - {} | {}", event2.source, event2.artist, event2.track, e),
            }
        });
    }

    if cfg.koito.forward_now_playing.unwrap_or(false) {
        let (cfg3, client3, event3) = (cfg.clone(), client.clone(), event.clone());
        tokio::spawn(async move {
            match koito::submit_now_playing(&cfg3.koito, &client3, &event3).await {
                Ok(()) => println!("[NOW] {} → Koito | {} - {}", event3.source, event3.artist, event3.track),
                Err(e) => eprintln!("[NOW-FAIL] {} → Koito now-playing | {} - {} | {}", event3.source, event3.artist, event3.track, e),
            }
        });
    }
}

async fn retry_log(target: &str, event: &PlayEvent, attempt: u32, e: &anyhow::Error) {
    let delays = [1u64, 4];
    if attempt < 3 {
        let delay = delays[(attempt - 1) as usize];
        eprintln!(
            "[FAIL] {} → {} | {} - {} | attempt {}/3 | {} | retrying in {}s",
            event.source, target, event.artist, event.track, attempt, e, delay
        );
        sleep(Duration::from_secs(delay)).await;
    } else {
        eprintln!(
            "[FAIL] {} → {} | {} - {} | attempt 3/3 | {}",
            event.source, target, event.artist, event.track, e
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{JellyfinConfig, KoitoConfig, LastFmConfig, ListenBrainzConfig, PlexConfig, ServerConfig};
    use crate::event::{NowPlayingEvent, Source};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingTarget {
        submit_calls: Arc<AtomicUsize>,
        submit_now_playing_calls: Arc<AtomicUsize>,
        fail_submit: bool,
    }

    #[async_trait]
    impl ScrobbleTarget for CountingTarget {
        fn name(&self) -> &'static str {
            "Counting"
        }

        async fn submit(&self, _event: &PlayEvent) -> anyhow::Result<()> {
            self.submit_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_submit {
                anyhow::bail!("forced failure");
            }
            Ok(())
        }

        async fn submit_now_playing(&self, _event: &NowPlayingEvent) -> anyhow::Result<()> {
            self.submit_now_playing_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct DefaultOnlyTarget;

    #[async_trait]
    impl ScrobbleTarget for DefaultOnlyTarget {
        fn name(&self) -> &'static str {
            "DefaultOnly"
        }

        async fn submit(&self, _event: &PlayEvent) -> anyhow::Result<()> {
            Ok(())
        }
        // submit_now_playing not overridden — uses the trait's default no-op
    }

    fn test_play_event() -> PlayEvent {
        PlayEvent {
            artist: "Test Artist".to_string(),
            album: None,
            track: "Test Track".to_string(),
            duration_secs: Some(200),
            played_at: chrono::Utc::now(),
            source: crate::event::Source::Navidrome,
        }
    }

    #[tokio::test]
    async fn default_submit_now_playing_returns_ok() {
        let target = DefaultOnlyTarget;
        let event = NowPlayingEvent {
            artist: "Test".to_string(),
            album: None,
            track: "Track".to_string(),
            duration_secs: None,
            source: crate::event::Source::Navidrome,
        };
        let result = target.submit_now_playing(&event).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn submit_with_retry_calls_submit_once_on_immediate_success() {
        let calls = Arc::new(AtomicUsize::new(0));
        let target = CountingTarget {
            submit_calls: calls.clone(),
            submit_now_playing_calls: Arc::new(AtomicUsize::new(0)),
            fail_submit: false,
        };
        let event = test_play_event();
        target.submit_with_retry(&event).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    fn minimal_cfg() -> Arc<Config> {
        Arc::new(Config {
            server: ServerConfig { port: 4567, webhook_token: None },
            plex: PlexConfig { webhook_token: None },
            jellyfin: JellyfinConfig { webhook_token: None },
            koito: KoitoConfig {
                base_url: "http://localhost:1".to_string(),
                api_key: "k".to_string(),
                forward_now_playing: Some(false),
            },
            listenbrainz: ListenBrainzConfig {
                user_token: "l".to_string(),
                forward_now_playing: Some(false),
            },
            lastfm: LastFmConfig {
                api_key: "a".to_string(),
                shared_secret: "s".to_string(),
                session_key: "k".to_string(),
                forward_now_playing: Some(false),
            },
        })
    }

    #[tokio::test]
    async fn fan_out_now_playing_does_not_panic_when_all_disabled() {
        let cfg = minimal_cfg();
        let client = reqwest::Client::new();
        let event = NowPlayingEvent {
            artist: "Test".to_string(),
            album: None,
            track: "Track".to_string(),
            duration_secs: None,
            source: Source::Navidrome,
        };
        // All targets disabled — should complete immediately without panicking
        fan_out_now_playing(cfg, client, event).await;
    }

    #[tokio::test]
    async fn fan_out_now_playing_spawns_when_enabled() {
        let cfg = Arc::new(Config {
            server: ServerConfig { port: 4567, webhook_token: None },
            plex: PlexConfig { webhook_token: None },
            jellyfin: JellyfinConfig { webhook_token: None },
            koito: KoitoConfig {
                base_url: "http://localhost:1".to_string(),
                api_key: "k".to_string(),
                forward_now_playing: Some(false),
            },
            listenbrainz: ListenBrainzConfig {
                user_token: "l".to_string(),
                forward_now_playing: Some(true),  // enabled
            },
            lastfm: LastFmConfig {
                api_key: "a".to_string(),
                shared_secret: "s".to_string(),
                session_key: "k".to_string(),
                forward_now_playing: Some(false),
            },
        });
        let client = reqwest::Client::new();
        let event = NowPlayingEvent {
            artist: "Test".to_string(),
            album: None,
            track: "Track".to_string(),
            duration_secs: None,
            source: Source::Navidrome,
        };
        // Should complete without panicking even though the spawned LB request will fail
        // (localhost:1 is unreachable — the spawn is fire-and-forget so this still returns)
        fan_out_now_playing(cfg, client, event).await;
    }
}
