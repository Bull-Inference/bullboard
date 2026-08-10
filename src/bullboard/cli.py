"""CLI entrypoint: `bullboard`."""

from __future__ import annotations

import argparse
import asyncio
import json
import sys

from bullboard import __version__
from bullboard.client import BullClient
from bullboard.config import API_BASE, X_HANDLE
from bullboard.xfeed import fetch_announce_feed


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="bullboard",
        description="Surfboard-style terminal dashboard for Bull.inf · $ANSEM · @blknoiz06",
    )
    p.add_argument("--version", action="version", version=f"bullboard {__version__}")
    p.add_argument(
        "--api-base",
        default=None,
        help=f"Bull API base (default {API_BASE} or $BULLBOARD_API_BASE)",
    )
    p.add_argument(
        "--handle",
        default=None,
        help=f"Primary X handle (default {X_HANDLE} or $BULLBOARD_X_HANDLE)",
    )
    p.add_argument(
        "--once",
        action="store_true",
        help="Fetch snapshot + tweets as JSON and exit (no TUI)",
    )
    p.add_argument(
        "--feed-mode",
        choices=("primary", "alt", "both"),
        default="primary",
        help="Announce feed mode for --once",
    )
    return p


async def _once(api_base: str | None, feed_mode: str) -> int:
    client = BullClient(api_base)
    snap = await client.fetch_all()
    tweets = await fetch_announce_feed(feed_mode, limit=12)
    out = {
        "version": __version__,
        "fetched_at": snap.fetched_at,
        "price_usd": snap.price_usd(),
        "change_24h": snap.change_24h(),
        "network": snap.network,
        "stats": {
            "requests": (snap.stats or {}).get("requests"),
            "live_models": (snap.stats or {}).get("live_models"),
            "activity_24h": (snap.stats or {}).get("activity_24h"),
            "liquidity_remaining": (snap.stats or {}).get("liquidity_remaining"),
        }
        if snap.stats
        else None,
        "stake": {
            "fees_routed_24h_ansem": (snap.stake or {}).get("fees_routed_24h_ansem"),
            "platform_fee": (snap.stake or {}).get("platform_fee"),
        }
        if snap.stake
        else None,
        "ohlc_stats": (snap.ohlc or {}).get("stats"),
        "feed_n": len((snap.feed or {}).get("feed") or []),
        "holders": {
            "holder_count": (snap.holders or {}).get("holder_count"),
            "holder_change_24h": (snap.holders or {}).get("holder_change_24h"),
            "top10_pct": (snap.holders or {}).get("top10_pct"),
            "rest_pct": (snap.holders or {}).get("rest_pct"),
            "traders_24h": (snap.holders or {}).get("traders_24h"),
            "circ_supply": (snap.holders or {}).get("circ_supply"),
            "top_holders": ((snap.holders or {}).get("top_holders") or [])[:3],
            "sources": (snap.holders or {}).get("sources"),
        }
        if snap.holders
        else None,
        "tweets": {
            "handle": tweets.get("handle"),
            "n": len(tweets.get("tweets") or []),
            "error": tweets.get("error"),
            "sample": (tweets.get("tweets") or [])[:3],
        },
        "errors": snap.errors,
    }
    print(json.dumps(out, indent=2, default=str))
    # success if we got a price, holders, or stats
    if snap.price_usd() is not None or snap.holders or snap.stats:
        return 0
    return 1


def main(argv: list[str] | None = None) -> None:
    args = _build_parser().parse_args(argv)

    if args.handle:
        import bullboard.config as cfg

        cfg.X_HANDLE = args.handle.lstrip("@")

    if args.once:
        code = asyncio.run(_once(args.api_base, args.feed_mode))
        raise SystemExit(code)

    try:
        from bullboard.app import run_app
    except ImportError as e:
        print(f"bullboard: TUI deps missing ({e}). Try: pip install 'bullboard[ ]' or pip install textual", file=sys.stderr)
        raise SystemExit(2) from e

    run_app(api_base=args.api_base)


if __name__ == "__main__":
    main()
