mod app;
mod config;
mod fetch;
mod format;
mod model;
mod ui;

use anyhow::Result;
use clap::Parser;
use config::Config;

#[derive(Parser, Debug)]
#[command(
    name = "bullboard",
    version,
    about = "Surfboard-style terminal dashboard for Bull.inf · $ANSEM · @blknoiz06"
)]
struct Cli {
    /// Bull API base URL
    #[arg(long, env = "BULLBOARD_API_BASE")]
    api_base: Option<String>,

    /// Primary X handle for announce feed
    #[arg(long, env = "BULLBOARD_X_HANDLE")]
    handle: Option<String>,

    /// Fetch snapshot JSON and exit (no TUI)
    #[arg(long)]
    once: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = Config::from_env();
    if let Some(b) = cli.api_base {
        cfg.api_base = b;
    }
    if let Some(h) = cli.handle {
        cfg.x_handle = h.trim_start_matches('@').to_string();
    }

    if cli.once {
        let json = fetch::once_json(&cfg).await?;
        println!("{json}");
        return Ok(());
    }

    app::run_tui(cfg).await
}
