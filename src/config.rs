use std::env;

pub const ANSEM_MINT: &str = "9cRCn9rGT8V2imeM2BaKs13yhMEais3ruM3rPvTGpump";
pub const DEFAULT_API: &str = "https://api.bullinf.fun";
pub const DEFAULT_HANDLE: &str = "blknoiz06";
pub const ALT_HANDLE: &str = "bullinference";

pub const REFRESH_DATA_SECS: u64 = 15;
pub const REFRESH_FEED_SECS: u64 = 60;

pub const NITTER_BASES: &[&str] = &[
    "https://nitter.net",
    "https://nitter.privacydev.net",
    "https://nitter.poast.org",
];

// Brand — Bull desk ink / acid, Surfboard-adjacent matrix
pub const ACID: ratatui::style::Color = ratatui::style::Color::Rgb(200, 245, 66);
pub const MUTED: ratatui::style::Color = ratatui::style::Color::Rgb(107, 111, 100);
pub const PANEL_BG: ratatui::style::Color = ratatui::style::Color::Rgb(12, 14, 11);
pub const BORDER: ratatui::style::Color = ratatui::style::Color::Rgb(42, 46, 36);
pub const FG: ratatui::style::Color = ratatui::style::Color::Rgb(200, 208, 184);
pub const TWEET_FG: ratatui::style::Color = ratatui::style::Color::Rgb(125, 222, 160);
pub const POST_FG: ratatui::style::Color = ratatui::style::Color::Rgb(80, 200, 120);
pub const WARN: ratatui::style::Color = ratatui::style::Color::Rgb(230, 184, 77);
pub const BAD: ratatui::style::Color = ratatui::style::Color::Rgb(227, 93, 93);

#[derive(Clone, Debug)]
pub struct Config {
    pub api_base: String,
    pub x_handle: String,
    pub x_handle_alt: String,
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
            x_handle_alt: env::var("BULLBOARD_X_HANDLE_ALT")
                .unwrap_or_else(|_| ALT_HANDLE.into())
                .trim_start_matches('@')
                .to_string(),
            mint: env::var("BULLBOARD_MINT").unwrap_or_else(|_| ANSEM_MINT.into()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedMode {
    Primary,
    Alt,
    Both,
}

impl FeedMode {
    pub fn next(self) -> Self {
        match self {
            Self::Primary => Self::Alt,
            Self::Alt => Self::Both,
            Self::Both => Self::Primary,
        }
    }

    pub fn label(self, cfg: &Config) -> String {
        match self {
            Self::Primary => cfg.x_handle.clone(),
            Self::Alt => cfg.x_handle_alt.clone(),
            Self::Both => format!("{}+{}", cfg.x_handle, cfg.x_handle_alt),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Alt => "alt",
            Self::Both => "both",
        }
    }
}
