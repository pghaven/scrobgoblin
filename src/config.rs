use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub koito: KoitoConfig,
    pub listenbrainz: ListenBrainzConfig,
    pub lastfm: LastFmConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub port: u16,
    /// If set, all Navidrome/ListenBrainz requests must present
    /// `Authorization: Token <webhook_token>`. Leave unset for
    /// internal-only deployments where the port is never exposed externally.
    pub webhook_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KoitoConfig {
    pub base_url: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListenBrainzConfig {
    pub user_token: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LastFmConfig {
    pub api_key: String,
    pub shared_secret: String,
    pub session_key: String,
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
        assert_eq!(cfg.koito.base_url, "http://koito.example.com");
        assert_eq!(cfg.listenbrainz.user_token, "lb-token");
        assert_eq!(cfg.lastfm.api_key, "lfm-key");
    }
}
