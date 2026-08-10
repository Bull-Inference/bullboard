"""Board snapshot model + signal derivation."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass
class BoardSnapshot:
    network: dict[str, Any] | None = None
    config: dict[str, Any] | None = None
    price: dict[str, Any] | None = None
    ohlc: dict[str, Any] | None = None
    stats: dict[str, Any] | None = None
    daily: dict[str, Any] | None = None
    stake: dict[str, Any] | None = None
    markets: list[dict[str, Any]] | None = None
    feed: dict[str, Any] | None = None
    tweets: dict[str, Any] | None = None
    errors: dict[str, str] = field(default_factory=dict)
    fetched_at: str | None = None
    feed_mode: str = "primary"

    def price_usd(self) -> float | None:
        if self.price and self.price.get("usd_price") is not None:
            return float(self.price["usd_price"])
        stats = (self.ohlc or {}).get("stats") or {}
        if stats.get("price_usd") is not None:
            return float(stats["price_usd"])
        if self.network and self.network.get("usd_price") is not None:
            return float(self.network["usd_price"])
        return None

    def change_24h(self) -> float | None:
        stats = (self.ohlc or {}).get("stats") or {}
        if stats.get("change_24h") is not None:
            return float(stats["change_24h"])
        return None

    def closes(self) -> list[float]:
        candles = (self.ohlc or {}).get("candles") or []
        out: list[float] = []
        for c in candles:
            try:
                out.append(float(c.get("close")))
            except (TypeError, ValueError, AttributeError):
                continue
        return out


def signals_from_snapshot(snap: BoardSnapshot) -> list[tuple[str, str, str]]:
    """Return list of (status, label, detail) where status is ok|warn|bad."""
    sigs: list[tuple[str, str, str]] = []
    net = snap.network or {}
    stats = snap.stats or {}
    stake = snap.stake or {}
    price = snap.price or {}
    errors = snap.errors or {}

    sol_ready = bool(net.get("solana_ready") or net.get("on_chain"))
    sigs.append(
        (
            "ok" if sol_ready else "bad",
            "SOLANA",
            "ready · on-chain settle" if sol_ready else "not ready",
        )
    )

    transfer = (net.get("transfer_mode") or "").lower()
    sigs.append(
        (
            "ok" if transfer == "solana" else "warn",
            "TRANSFER",
            transfer or "unknown",
        )
    )

    if price.get("stale"):
        sigs.append(("warn", "PRICE", "stale quote"))
    elif snap.price_usd() is not None:
        src = price.get("source") or "live"
        sigs.append(("ok", "PRICE", f"{src} feed"))
    else:
        sigs.append(("bad", "PRICE", "no quote"))

    live_models = stats.get("live_models") or stats.get("open_offers") or 0
    liq = stats.get("liquidity_remaining")
    if live_models:
        sigs.append(("ok", "MARKETS", f"{live_models} models · liq {liq if liq is not None else '—'}"))
    else:
        sigs.append(("warn", "MARKETS", "empty board"))

    act = (stats.get("activity_24h") or {}).get("requests")
    if act is None:
        sigs.append(("warn", "ACTIVITY 24h", "no data"))
    elif act > 0:
        sigs.append(("ok", "ACTIVITY 24h", f"{act} requests"))
    else:
        sigs.append(("warn", "ACTIVITY 24h", "quiet"))

    if stake:
        fees = stake.get("fees_routed_24h_ansem")
        sigs.append(("ok", "STAKE FEES", f"24h routed {fees if fees is not None else '—'} ANSEM"))
    else:
        sigs.append(("warn", "STAKE", "no stats"))

    for key, err in list(errors.items())[:4]:
        if err:
            sigs.append(("bad", key.upper(), err[:48]))

    return sigs
