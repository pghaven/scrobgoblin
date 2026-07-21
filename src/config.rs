use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(default)]
    pub plex: PlexConfig,
    #[serde(default)]
    pub jellyfin: JellyfinConfig,
    #[serde(default)]
    pub koito: Option<KoitoConfig>,
    #[serde(default)]
    pub listenbrainz: Option<ListenBrainzConfig>,
    #[serde(default)]
    pub lastfm: Option<LastFmConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub port: u16,
    /// If set, all Navidrome/ListenBrainz requests must present
    /// `Authorization: Token <webhook_token>`. Leave unset for
    /// internal-only deployments where the port is never exposed externally.
    pub webhook_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PlexConfig {
    pub webhook_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct JellyfinConfig {
    pub webhook_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KoitoConfig {
    pub base_url: String,
    pub api_key: String,
    pub forward_now_playing: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListenBrainzConfig {
    pub user_token: String,
    pub forward_now_playing: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LastFmConfig {
    pub api_key: String,
    pub shared_secret: String,
    pub session_key: String,
    pub forward_now_playing: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_config() {
        let toml = r#"
[server]
port = 4567

[koito]
base_url = "http://koito.example.com"
api_key = "koito-key"

[listenbrainz]
user_token = "lb-token"

[lastfm]
api_key = "lfm-key"
shared_secret = "lfm-secret"
session_key = "lfm-session"
"#;
        let cfg: Config = toml::from_str(toml).expect("should parse");
        assert_eq!(cfg.server.port, 4567);
        assert_eq!(cfg.koito.unwrap().base_url, "http://koito.example.com");
        assert_eq!(cfg.listenbrainz.unwrap().user_token, "lb-token");
        assert_eq!(cfg.lastfm.unwrap().api_key, "lfm-key");
    }

    #[test]
    fn parses_forward_now_playing_flags() {
        let toml = r#"
[server]
port = 4567

[koito]
base_url = "http://koito.example.com"
api_key = "koito-key"
forward_now_playing = true

[listenbrainz]
user_token = "lb-token"
forward_now_playing = false

[lastfm]
api_key = "lfm-key"
shared_secret = "lfm-secret"
session_key = "lfm-session"
"#;
        let cfg: Config = toml::from_str(toml).expect("should parse");
        assert_eq!(cfg.koito.unwrap().forward_now_playing, Some(true));
        assert_eq!(cfg.listenbrainz.unwrap().forward_now_playing, Some(false));
        assert_eq!(cfg.lastfm.unwrap().forward_now_playing, None); // omitted → None
    }

    #[test]
    fn parses_plex_and_jellyfin_auth_config() {
        let toml = r#"
[server]
port = 4567

[plex]
webhook_token = "plex-secret"

[jellyfin]
webhook_token = "jf-secret"

[koito]
base_url = "http://koito.example.com"
api_key = "koito-key"

[listenbrainz]
user_token = "lb-token"

[lastfm]
api_key = "lfm-key"
shared_secret = "lfm-secret"
session_key = "lfm-session"
"#;
        let cfg: Config = toml::from_str(toml).expect("should parse");
        assert_eq!(cfg.plex.webhook_token.as_deref(), Some("plex-secret"));
        assert_eq!(cfg.jellyfin.webhook_token.as_deref(), Some("jf-secret"));
    }

    #[test]
    fn plex_and_jellyfin_default_to_no_token_when_section_absent() {
        let toml = r#"
[server]
port = 4567

[koito]
base_url = "http://koito.example.com"
api_key = "koito-key"

[listenbrainz]
user_token = "lb-token"

[lastfm]
api_key = "lfm-key"
shared_secret = "lfm-secret"
session_key = "lfm-session"
"#;
        let cfg: Config = toml::from_str(toml).expect("should parse");
        assert!(cfg.plex.webhook_token.is_none());
        assert!(cfg.jellyfin.webhook_token.is_none());
    }

    #[test]
    fn target_sections_are_all_optional() {
        let toml = r#"
[server]
port = 4567
"#;
        let cfg: Config = toml::from_str(toml).expect("should parse");
        assert!(cfg.koito.is_none());
        assert!(cfg.listenbrainz.is_none());
        assert!(cfg.lastfm.is_none());
    }

    #[test]
    fn partial_target_configuration_parses() {
        let toml = r#"
[server]
port = 4567

[koito]
base_url = "http://koito.example.com"
api_key = "koito-key"

[listenbrainz]
user_token = "lb-token"
"#;
        let cfg: Config = toml::from_str(toml).expect("should parse");
        assert!(cfg.koito.is_some());
        assert!(cfg.listenbrainz.is_some());
        assert!(cfg.lastfm.is_none());
    }
}
