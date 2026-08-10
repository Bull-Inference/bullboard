use chrono::{DateTime, Local, Utc};

const SPARK: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

pub fn fmt_usd(x: Option<f64>) -> String {
    match x {
        None => "—".into(),
        Some(v) if v.abs() >= 1_000_000.0 => format!("${:.2}M", v / 1_000_000.0),
        Some(v) if v.abs() >= 1_000.0 => format!("${:.2}K", v / 1_000.0),
        Some(v) if v.abs() >= 1.0 => {
            let s = format!("${v:.4}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        Some(v) => format!("${v:.4}"),
    }
}

pub fn fmt_compact(x: Option<f64>) -> String {
    match x {
        None => "—".into(),
        Some(v) => {
            let sign = if v < 0.0 { "-" } else { "" };
            let v = v.abs();
            if v >= 1_000_000_000.0 {
                format!("{sign}{:.2}B", v / 1_000_000_000.0)
            } else if v >= 1_000_000.0 {
                format!("{sign}{:.2}M", v / 1_000_000.0)
            } else if v >= 1_000.0 {
                format!("{sign}{:.2}K", v / 1_000.0)
            } else if v >= 100.0 {
                format!("{sign}{v:.0}")
            } else if v >= 1.0 {
                let s = format!("{sign}{v:.2}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            } else {
                format!("{sign}{v:.4}")
            }
        }
    }
}

pub fn fmt_ansem(x: Option<f64>) -> String {
    format!("{} ANSEM", fmt_compact(x))
}

pub fn fmt_int(n: Option<u64>) -> String {
    match n {
        None => "—".into(),
        Some(v) => {
            let s = v.to_string();
            let mut out = String::new();
            for (i, c) in s.chars().rev().enumerate() {
                if i > 0 && i % 3 == 0 {
                    out.push(',');
                }
                out.push(c);
            }
            out.chars().rev().collect()
        }
    }
}

pub fn short_addr(s: &str, n: usize) -> String {
    if s.is_empty() {
        return "—".into();
    }
    if s.len() <= n * 2 + 1 {
        return s.to_string();
    }
    format!("{}…{}", &s[..n], &s[s.len() - n..])
}

pub fn delta_str(pct: Option<f64>) -> String {
    match pct {
        None => "—".into(),
        Some(p) => {
            let arrow = if p >= 0.0 { "▲" } else { "▼" };
            format!("{arrow} {p:+.2}%")
        }
    }
}

pub fn sparkline(closes: &[f64], width: usize) -> String {
    if closes.is_empty() || width == 0 {
        return "─".repeat(width.min(8).max(1));
    }
    let vals: Vec<f64> = if closes.len() > width {
        let step = closes.len() as f64 / width as f64;
        (0..width)
            .map(|i| closes[(i as f64 * step) as usize])
            .collect()
    } else {
        closes.to_vec()
    };
    let lo = vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let span = if (hi - lo).abs() < f64::EPSILON {
        1.0
    } else {
        hi - lo
    };
    vals.iter()
        .map(|v| {
            let idx = (((v - lo) / span) * (SPARK.len() - 1) as f64).round() as usize;
            SPARK[idx.min(SPARK.len() - 1)]
        })
        .collect()
}

pub fn parse_iso(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
        .or_else(|| {
            let cleaned = s.replace('Z', "+00:00");
            DateTime::parse_from_rfc3339(&cleaned)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        })
}

pub fn ago(iso: Option<&str>) -> String {
    let Some(s) = iso else {
        return "—".into();
    };
    let Some(dt) = parse_iso(s) else {
        return "—".into();
    };
    let sec = (Utc::now() - dt).num_seconds().max(0);
    if sec < 60 {
        format!("{sec}s ago")
    } else if sec < 3600 {
        format!("{}m ago", sec / 60)
    } else if sec < 86400 {
        format!("{}h ago", sec / 3600)
    } else {
        format!("{}d ago", sec / 86400)
    }
}

pub fn clock_mmdd_hhmm(iso: Option<&str>) -> String {
    let Some(s) = iso else {
        return "??-?? ??:??".into();
    };
    let Some(dt) = parse_iso(s) else {
        return "??-?? ??:??".into();
    };
    let local = dt.with_timezone(&Local);
    local.format("%m-%d %H:%M").to_string()
}

pub fn age_from_ms(ms: Option<u64>) -> String {
    let Some(ms) = ms else {
        return "—".into();
    };
    let now_ms = Utc::now().timestamp_millis().max(0) as u64;
    let secs = now_ms.saturating_sub(ms) / 1000;
    if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Terminal-safe bar using half-width ASCII so columns stay aligned.
pub fn bar(pct: Option<f64>, width: usize) -> String {
    let width = width.max(4);
    let p = pct.unwrap_or(0.0).clamp(0.0, 100.0);
    let filled = ((p / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    format!(
        "[{}{}]",
        "#".repeat(filled),
        "-".repeat(width.saturating_sub(filled))
    )
}

pub fn pad_label(label: &str, width: usize) -> String {
    let mut s = label.to_string();
    while s.chars().count() < width {
        s.push(' ');
    }
    if s.chars().count() > width {
        s.chars().take(width).collect()
    } else {
        s
    }
}
