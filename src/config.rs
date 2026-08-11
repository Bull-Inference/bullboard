use std::env;

pub const ANSEM_MINT: &str = "9cRCn9rGT8V2imeM2BaKs13yhMEais3ruM3rPvTGpump";
pub const DEFAULT_API: &str = "https://api.bullinf.fun";
pub const DEFAULT_HANDLE: &str = "blknoiz06";

pub const REFRESH_DATA_SECS: u64 = 15;
pub const REFRESH_FEED_SECS: u64 = 30;

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

// Per-pane accents — muted hues so the board reads as one acid-on-dark
// family, each pane with its own quiet identity.
pub const CYAN: ratatui::style::Color = ratatui::style::Color::Rgb(94, 224, 214);
pub const VIOLET: ratatui::style::Color = ratatui::style::Color::Rgb(186, 158, 255);
pub const BLUE: ratatui::style::Color = ratatui::style::Color::Rgb(116, 170, 255);
pub const PURPLE: ratatui::style::Color = ratatui::style::Color::Rgb(224, 124, 226);

#[derive(Clone, Debug)]
pub struct Config {
    pub api_base: String,
    pub x_handle: String,
    pub mint: String,
    /// Initial desktop-notify state (BULLBOARD_NOTIFY=1); `t` toggles at runtime.
    pub notify: bool,
    /// Bypass the mirrors' 10-minute RSS cache with a live fetch per poll
    /// (BULLBOARD_FRESH_FEED=0 disables — polite mode, ~10 min delayed).
    pub fresh_feed: bool,
    /// Announce feed poll interval in seconds (BULLBOARD_FEED_SECS).
    pub feed_secs: u64,
    /// Nitter mirrors for the announce feed (BULLBOARD_MIRRORS,
    /// comma-separated — no recompile needed when one starts blocking).
    pub mirrors: Vec<String>,
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
            notify: env::var("BULLBOARD_NOTIFY")
                .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
                .unwrap_or(false),
            fresh_feed: env::var("BULLBOARD_FRESH_FEED")
                .map(|v| !matches!(v.to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"))
                .unwrap_or(true),
            feed_secs: env::var("BULLBOARD_FEED_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .map(|s: u64| s.max(5))
                .unwrap_or(REFRESH_FEED_SECS),
            mirrors: env::var("BULLBOARD_MIRRORS")
                .ok()
                .map(|s| {
                    s.split(',')
                        .map(str::trim)
                        .filter(|m| !m.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .filter(|v: &Vec<String>| !v.is_empty())
                .unwrap_or_else(|| NITTER_BASES.iter().map(|s| s.to_string()).collect()),
        }
    }
}
