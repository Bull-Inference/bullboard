"""Display helpers — compact numbers, deltas, sparklines."""

from __future__ import annotations

from datetime import datetime, timezone
from typing import Iterable

_SPARK = "▁▂▃▄▅▆▇█"


def fmt_usd(x: float | int | None, *, digits: int = 4) -> str:
    if x is None:
        return "—"
    try:
        v = float(x)
    except (TypeError, ValueError):
        return "—"
    if abs(v) >= 1_000_000:
        return f"${v / 1_000_000:.2f}M"
    if abs(v) >= 1_000:
        return f"${v / 1_000:.2f}K"
    if abs(v) >= 1:
        return f"${v:.4f}".rstrip("0").rstrip(".")
    return f"${v:.{digits}f}"


def fmt_pct(x: float | int | None, *, digits: int = 2) -> str:
    if x is None:
        return "—"
    try:
        return f"{float(x):+.{digits}f}%"
    except (TypeError, ValueError):
        return "—"


def fmt_compact(n: float | int | None) -> str:
    if n is None:
        return "—"
    try:
        v = float(n)
    except (TypeError, ValueError):
        return "—"
    sign = "-" if v < 0 else ""
    v = abs(v)
    if v >= 1_000_000_000:
        return f"{sign}{v / 1_000_000_000:.2f}B"
    if v >= 1_000_000:
        return f"{sign}{v / 1_000_000:.2f}M"
    if v >= 1_000:
        return f"{sign}{v / 1_000:.2f}K"
    if v >= 100:
        return f"{sign}{v:.0f}"
    if v >= 1:
        return f"{sign}{v:.2f}".rstrip("0").rstrip(".")
    return f"{sign}{v:.4f}".rstrip("0").rstrip(".")


def fmt_ansem(x: float | int | None) -> str:
    if x is None:
        return "—"
    return f"{fmt_compact(x)} ANSEM"


def short_addr(s: str | None, n: int = 4) -> str:
    if not s:
        return "—"
    s = str(s)
    if len(s) <= n * 2 + 1:
        return s
    return f"{s[:n]}…{s[-n:]}"


def _parse_dt(iso_or_dt: str | datetime | None) -> datetime | None:
    if iso_or_dt is None:
        return None
    if isinstance(iso_or_dt, datetime):
        dt = iso_or_dt
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        return dt
    raw = str(iso_or_dt).strip()
    if not raw:
        return None
    if raw.endswith("Z"):
        raw = raw[:-1] + "+00:00"
    try:
        dt = datetime.fromisoformat(raw)
    except ValueError:
        return None
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt


def ago(iso_or_dt: str | datetime | None) -> str:
    dt = _parse_dt(iso_or_dt)
    if dt is None:
        return "—"
    now = datetime.now(timezone.utc)
    sec = int((now - dt).total_seconds())
    if sec < 0:
        sec = 0
    if sec < 60:
        return f"{sec}s ago"
    if sec < 3600:
        return f"{sec // 60}m ago"
    if sec < 86400:
        return f"{sec // 3600}h ago"
    return f"{sec // 86400}d ago"


def clock_mmdd_hhmm(iso_or_dt: str | datetime | None) -> str:
    dt = _parse_dt(iso_or_dt)
    if dt is None:
        return "??-?? ??:??"
    local = dt.astimezone()
    return local.strftime("%m-%d %H:%M")


def sparkline(closes: Iterable[float] | None, width: int = 24) -> str:
    if not closes:
        return "─" * min(width, 8)
    vals = [float(c) for c in closes if c is not None]
    if not vals:
        return "─" * min(width, 8)
    if len(vals) > width:
        # downsample
        step = len(vals) / width
        vals = [vals[int(i * step)] for i in range(width)]
    lo, hi = min(vals), max(vals)
    span = hi - lo or 1.0
    out = []
    for v in vals:
        idx = int((v - lo) / span * (len(_SPARK) - 1))
        out.append(_SPARK[idx])
    return "".join(out)


def delta_str(pct: float | int | None) -> str:
    if pct is None:
        return "—"
    try:
        p = float(pct)
    except (TypeError, ValueError):
        return "—"
    arrow = "▲" if p >= 0 else "▼"
    return f"{arrow} {p:+.2f}%"
