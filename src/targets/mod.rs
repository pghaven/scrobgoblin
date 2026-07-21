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

pub fn build_targets(cfg: &Config, client: reqwest::Client) -> Vec<Arc<dyn ScrobbleTarget>> {
    let mut targets: Vec<Arc<dyn ScrobbleTarget>> = Vec::new();
    if let Some(k) = &cfg.koito {
        targets.push(Arc::new(koito::KoitoTarget::from_config(k, client.clone())));
    }
    if let Some(lb) = &cfg.listenbrainz {
        targets.push(Arc::new(listenbrainz::ListenBrainzTarget::from_config(lb, client.clone())));
    }
    if let Some(lfm) = &cfg.lastfm {
        targets.push(Arc::new(lastfm::LastFmTarget::from_config(lfm, client.clone())));
    }
    targets
}

async fn dispatch<E, F, Fut>(targets: Vec<Arc<dyn ScrobbleTarget>>, event: E, join: bool, call: F)
where
    E: Send + Sync + 'static,
    F: Fn(Arc<dyn ScrobbleTarget>, Arc<E>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let event = Arc::new(event);
    let handles: Vec<_> = targets
        .into_iter()
        .map(|t| {
            let event = event.clone();
            tokio::spawn(call(t, event))
        })
        .collect();
    if join {
        for h in handles {
            let _ = h.await;
        }
    }
}

pub async fn fan_out(targets: Vec<Arc<dyn ScrobbleTarget>>, event: PlayEvent) {
    dispatch(targets, event, true, |t, event| async move {
        t.submit_with_retry(&event).await;
    })
    .await;
}

pub async fn fan_out_now_playing(targets: Vec<Arc<dyn ScrobbleTarget>>, event: NowPlayingEvent) {
    dispatch(targets, event, false, |t, event| async move {
        if let Err(e) = t.submit_now_playing(&event).await {
            eprintln!("[NOW-FAIL] {} → {} | {}", event.source, t.name(), e);
        }
    })
    .await;
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

    #[tokio::test]
    async fn fan_out_now_playing_does_not_panic_when_all_disabled() {
        let cfg = Arc::new(Config {
            server: ServerConfig { port: 4567, webhook_token: None },
            plex: PlexConfig { webhook_token: None },
            jellyfin: JellyfinConfig { webhook_token: None },
            koito: Some(KoitoConfig {
                base_url: "http://localhost:1".to_string(),
                api_key: "k".to_string(),
                forward_now_playing: Some(false),
            }),
            listenbrainz: Some(ListenBrainzConfig {
                user_token: "l".to_string(),
                forward_now_playing: Some(false),
            }),
            lastfm: Some(LastFmConfig {
                api_key: "a".to_string(),
                shared_secret: "s".to_string(),
                session_key: "k".to_string(),
                forward_now_playing: Some(false),
            }),
        });
        let targets = build_targets(&cfg, reqwest::Client::new());
        let event = NowPlayingEvent {
            artist: "Test".to_string(),
            album: None,
            track: "Track".to_string(),
            duration_secs: None,
            source: Source::Navidrome,
        };
        fan_out_now_playing(targets, event).await;
    }

    #[tokio::test]
    async fn fan_out_now_playing_spawns_when_enabled() {
        let cfg = Arc::new(Config {
            server: ServerConfig { port: 4567, webhook_token: None },
            plex: PlexConfig { webhook_token: None },
            jellyfin: JellyfinConfig { webhook_token: None },
            koito: Some(KoitoConfig {
                base_url: "http://localhost:1".to_string(),
                api_key: "k".to_string(),
                forward_now_playing: Some(false),
            }),
            listenbrainz: Some(ListenBrainzConfig {
                user_token: "l".to_string(),
                forward_now_playing: Some(true), // enabled
            }),
            lastfm: Some(LastFmConfig {
                api_key: "a".to_string(),
                shared_secret: "s".to_string(),
                session_key: "k".to_string(),
                forward_now_playing: Some(false),
            }),
        });
        let targets = build_targets(&cfg, reqwest::Client::new());
        let event = NowPlayingEvent {
            artist: "Test".to_string(),
            album: None,
            track: "Track".to_string(),
            duration_secs: None,
            source: Source::Navidrome,
        };
        // Should complete without panicking even though the spawned LB request will fail
        // (localhost:1 is unreachable — the spawn is fire-and-forget so this still returns)
        fan_out_now_playing(targets, event).await;
    }

    #[tokio::test]
    async fn fan_out_joins_all_spawned_tasks() {
        let calls_a = Arc::new(AtomicUsize::new(0));
        let calls_b = Arc::new(AtomicUsize::new(0));
        let targets: Vec<Arc<dyn ScrobbleTarget>> = vec![
            Arc::new(CountingTarget {
                submit_calls: calls_a.clone(),
                submit_now_playing_calls: Arc::new(AtomicUsize::new(0)),
                fail_submit: false,
            }),
            Arc::new(CountingTarget {
                submit_calls: calls_b.clone(),
                submit_now_playing_calls: Arc::new(AtomicUsize::new(0)),
                fail_submit: false,
            }),
        ];
        fan_out(targets, test_play_event()).await;
        assert_eq!(calls_a.load(Ordering::SeqCst), 1);
        assert_eq!(calls_b.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn build_targets_skips_unconfigured_lastfm() {
        let cfg = Config {
            server: ServerConfig { port: 4567, webhook_token: None },
            plex: PlexConfig { webhook_token: None },
            jellyfin: JellyfinConfig { webhook_token: None },
            koito: Some(KoitoConfig {
                base_url: "http://k".to_string(),
                api_key: "k".to_string(),
                forward_now_playing: None,
            }),
            listenbrainz: Some(ListenBrainzConfig {
                user_token: "l".to_string(),
                forward_now_playing: None,
            }),
            lastfm: None,
        };
        let targets = build_targets(&cfg, reqwest::Client::new());
        assert_eq!(targets.len(), 2);
        let names: Vec<&str> = targets.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"Koito"));
        assert!(names.contains(&"ListenBrainz"));
        assert!(!names.contains(&"Last.fm"));
    }

    #[test]
    fn build_targets_returns_empty_when_none_configured() {
        let cfg = Config {
            server: ServerConfig { port: 4567, webhook_token: None },
            plex: PlexConfig { webhook_token: None },
            jellyfin: JellyfinConfig { webhook_token: None },
            koito: None,
            listenbrainz: None,
            lastfm: None,
        };
        let targets = build_targets(&cfg, reqwest::Client::new());
        assert!(targets.is_empty());
    }
}
