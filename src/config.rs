use std::env;

pub const ANSEM_MINT: &str = "9cRCn9rGT8V2imeM2BaKs13yhMEais3ruM3rPvTGpump";
pub const DEFAULT_API: &str = "https://api.bullinf.fun";
pub const DEFAULT_HANDLE: &str = "blknoiz06";

pub const REFRESH_DATA_SECS: u64 = 15;
pub const REFRESH_FEED_SECS: u64 = 60;

pub const NITTER_BASES: &[&str] = &[
    "https://nitter.net",
    "https://nitter.privacydev.net",
    "https://nitter.poast.org",
];

// Brand — Bull desk ink / acid, Surfboard matrix
// Canvas is darker than panels so gutters read as intentional dark air.
pub const ACID: ratatui::style::Color = ratatui::style::Color::Rgb(200, 245, 66);
pub const MUTED: ratatui::style::Color = ratatui::style::Color::Rgb(107, 111, 100);
pub const CANVAS_BG: ratatui::style::Color = ratatui::style::Color::Rgb(5, 6, 4);
pub const PANEL_BG: ratatui::style::Color = ratatui::style::Color::Rgb(22, 24, 18);
pub const BORDER: ratatui::style::Color = ratatui::style::Color::Rgb(48, 52, 40);
pub const FG: ratatui::style::Color = ratatui::style::Color::Rgb(200, 208, 184);
pub const TWEET_FG: ratatui::style::Color = ratatui::style::Color::Rgb(125, 222, 160);
pub const POST_FG: ratatui::style::Color = ratatui::style::Color::Rgb(80, 200, 120);
pub const WARN: ratatui::style::Color = ratatui::style::Color::Rgb(230, 184, 77);
pub const BAD: ratatui::style::Color = ratatui::style::Color::Rgb(227, 93, 93);

#[derive(Clone, Debug)]
pub struct Config {
    pub api_base: String,
    pub x_handle: String,
    pub mint: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            api_base: env::var("BULLBOARD_API_BASE").unwrap_or_else(|_| DEFAULT_API.into()),
            x_handle: env::var("BULLBOARD_X_HANDLE")
                .unwrap_or_else(|_| DEFAULT_HANDLE.into())
                .trim_start_matches('@')
                .to_string(),
            mint: env::var("BULLBOARD_MINT").unwrap_or_else(|_| ANSEM_MINT.into()),
        }
    }
}

/// Announce feed source — $ANSEM-related X only (no inference alt handle).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FeedMode {
    #[default]
    Primary,
}

impl FeedMode {
    pub fn label(self, cfg: &Config) -> String {
        cfg.x_handle.clone()
    }
}
