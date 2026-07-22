mod config;
mod event;
mod router;
mod sources;
mod targets;
mod threshold;

use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_str = std::fs::read_to_string("config.toml")
        .map_err(|e| anyhow::anyhow!("failed to read config.toml: {}", e))?;
    let cfg: config::Config =
        toml::from_str(&config_str).map_err(|e| anyhow::anyhow!("invalid config.toml: {}", e))?;

    let port = cfg.server.port;
    let client = reqwest::Client::new();
    let targets = targets::build_targets(&cfg, client.clone());
    if targets.is_empty() {
        eprintln!("[WARN] No scrobble targets configured — check config.toml section names ([koito]/[listenbrainz]/[lastfm])");
    } else {
        let names: Vec<&str> = targets.iter().map(|t| t.name()).collect();
        println!("Active scrobble targets: {}", names.join(", "));
    }
    let cfg = Arc::new(cfg);

    let state = router::AppState { cfg, targets };
    let app = router::build_router(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    println!("Scrobgoblin listening on 0.0.0.0:{}", port);
    axum::serve(listener, app).await?;
    Ok(())
}
