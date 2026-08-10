"""Nitter RSS announce feed for @blknoiz06 (and alts)."""

from __future__ import annotations

import asyncio
import html as html_lib
import re
import time
from datetime import datetime, timezone
from email.utils import parsedate_to_datetime
from typing import Any
from xml.etree import ElementTree as ET

import httpx

from bullboard.config import NITTER_BASES, SECOND_X_HANDLE, X_HANDLE

_TAG_RE = re.compile(r"<[^>]+>")
_STATUS_RE = re.compile(r"/status/(\d+)")
_CACHE_TTL = 60.0
_cache: dict[str, tuple[float, dict[str, Any]]] = {}
_TIMEOUT = httpx.Timeout(10.0, connect=6.0)


def _now_iso() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def _clean_text(raw: str) -> str:
    text = html_lib.unescape(raw or "")
    text = _TAG_RE.sub(" ", text)
    # nitter artifact tails
    text = re.sub(r"https?://nitter\.[^\s]+", "", text)
    text = re.sub(r"#m\b", "", text)
    text = re.sub(r"\s+", " ", text).strip()
    return text


def _item_id(link: str, guid: str | None = None) -> str:
    for candidate in (link, guid or ""):
        m = _STATUS_RE.search(candidate or "")
        if m:
            return m.group(1)
    return (link or guid or "unknown")[-16:]


def _parse_rss(body: bytes, handle: str) -> list[dict[str, Any]]:
    root = ET.fromstring(body)
    tweets: list[dict[str, Any]] = []
    for item in root.findall(".//item"):
        title = (item.findtext("title") or "").strip()
        desc = (item.findtext("description") or "").strip()
        link = (item.findtext("link") or "").strip()
        guid = (item.findtext("guid") or "").strip()
        pub = (item.findtext("pubDate") or "").strip()
        text = _clean_text(desc or title)
        if not text:
            continue
        created_at = None
        if pub:
            try:
                dt = parsedate_to_datetime(pub)
                if dt.tzinfo is None:
                    dt = dt.replace(tzinfo=timezone.utc)
                created_at = dt.astimezone(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
            except (TypeError, ValueError, IndexError):
                created_at = None
        tid = _item_id(link, guid)
        if not link:
            link = f"https://x.com/{handle}/status/{tid}"
        tweets.append(
            {
                "id": tid,
                "text": text,
                "created_at": created_at,
                "url": link.replace("nitter.net", "x.com").replace("nitter.privacydev.net", "x.com"),
            }
        )
    return tweets


async def _fetch_from_base(base: str, handle: str) -> list[dict[str, Any]]:
    url = f"{base.rstrip('/')}/{handle}/rss"
    async with httpx.AsyncClient(timeout=_TIMEOUT, headers={"User-Agent": "bullboard/0.1"}, follow_redirects=True) as client:
        r = await client.get(url)
        r.raise_for_status()
        return _parse_rss(r.content, handle)


async def fetch_tweets(handle: str, *, limit: int = 20, force: bool = False) -> dict[str, Any]:
    handle = (handle or "").lstrip("@").strip()
    if not handle:
        return {
            "handle": "",
            "profile_url": "",
            "tweets": [],
            "source": "error",
            "error": "empty handle",
            "fetched_at": _now_iso(),
        }

    cache_key = handle.lower()
    now = time.time()
    if not force and cache_key in _cache:
        ts, payload = _cache[cache_key]
        if now - ts < _CACHE_TTL:
            out = dict(payload)
            out["tweets"] = list(payload.get("tweets") or [])[:limit]
            out["cached"] = True
            return out

    errors: list[str] = []
    for base in NITTER_BASES:
        try:
            tweets = await _fetch_from_base(base, handle)
            payload = {
                "handle": handle,
                "profile_url": f"https://x.com/{handle}",
                "tweets": tweets[:limit],
                "source": base,
                "error": None,
                "fetched_at": _now_iso(),
                "cached": False,
            }
            _cache[cache_key] = (now, {**payload, "tweets": tweets})
            return payload
        except Exception as e:  # noqa: BLE001
            errors.append(f"{base}: {type(e).__name__}")
            continue

    stale = _cache.get(cache_key)
    if stale:
        ts, payload = stale
        out = dict(payload)
        out["tweets"] = list(payload.get("tweets") or [])[:limit]
        out["error"] = "; ".join(errors)[:160]
        out["source"] = "stale-cache"
        out["cached"] = True
        out["fetched_at"] = _now_iso()
        return out

    return {
        "handle": handle,
        "profile_url": f"https://x.com/{handle}",
        "tweets": [],
        "source": "error",
        "error": "; ".join(errors)[:160] or "all nitter bases failed",
        "fetched_at": _now_iso(),
        "cached": False,
    }


async def fetch_announce_feed(mode: str = "primary", *, limit: int = 20) -> dict[str, Any]:
    mode = (mode or "primary").lower()
    if mode == "alt":
        return await fetch_tweets(SECOND_X_HANDLE, limit=limit)
    if mode == "both":
        a, b = await asyncio.gather(
            fetch_tweets(X_HANDLE, limit=limit),
            fetch_tweets(SECOND_X_HANDLE, limit=limit),
        )
        merged: list[dict[str, Any]] = []
        for t in (a.get("tweets") or []):
            merged.append({**t, "handle": a.get("handle")})
        for t in (b.get("tweets") or []):
            merged.append({**t, "handle": b.get("handle")})
        merged.sort(key=lambda t: t.get("created_at") or "", reverse=True)
        return {
            "handle": f"{X_HANDLE}+{SECOND_X_HANDLE}",
            "profile_url": f"https://x.com/{X_HANDLE}",
            "tweets": merged[:limit],
            "source": "both",
            "error": a.get("error") or b.get("error"),
            "fetched_at": _now_iso(),
            "parts": [a, b],
        }
    return await fetch_tweets(X_HANDLE, limit=limit)


def fetch_tweets_sync(handle: str, *, limit: int = 20) -> dict[str, Any]:
    return asyncio.run(fetch_tweets(handle, limit=limit))
