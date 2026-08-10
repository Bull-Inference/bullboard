"""Async Bull.inf public API client."""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone
from typing import Any

import httpx

from bullboard.config import ANSEM_MINT, API_BASE
from bullboard.models import BoardSnapshot

_TIMEOUT = httpx.Timeout(12.0, connect=8.0)
_UA = {"User-Agent": "bullboard/0.1", "Accept": "application/json"}


def _now_iso() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


class BullClient:
    def __init__(self, base: str | None = None) -> None:
        self.base = (base or API_BASE).rstrip("/")

    async def _get(
        self,
        path: str,
        params: dict[str, Any] | None = None,
        *,
        absolute: bool = False,
    ) -> tuple[Any | None, str | None]:
        url = path if absolute else f"{self.base}{path}"
        try:
            async with httpx.AsyncClient(timeout=_TIMEOUT, headers=_UA) as client:
                r = await client.get(url, params=params)
                if r.status_code >= 400:
                    return None, f"HTTP {r.status_code}"
                return r.json(), None
        except httpx.TimeoutException:
            return None, "timeout"
        except Exception as e:  # noqa: BLE001 — never raise into TUI
            return None, f"{type(e).__name__}: {e}"[:120]

    async def get_network(self) -> tuple[dict | None, str | None]:
        data, err = await self._get("/api/network")
        return (data if isinstance(data, dict) else None), err

    async def get_config(self) -> tuple[dict | None, str | None]:
        data, err = await self._get("/api/config")
        return (data if isinstance(data, dict) else None), err

    async def get_token_price(self) -> tuple[dict | None, str | None]:
        data, err = await self._get("/api/token-price")
        return (data if isinstance(data, dict) else None), err

    async def get_token_ohlc(self, interval: str = "1h", limit: int = 48) -> tuple[dict | None, str | None]:
        data, err = await self._get("/api/token-ohlc", {"interval": interval, "limit": limit})
        return (data if isinstance(data, dict) else None), err

    async def get_stats(self) -> tuple[dict | None, str | None]:
        data, err = await self._get("/api/stats")
        return (data if isinstance(data, dict) else None), err

    async def get_stats_daily(self, days: int = 14) -> tuple[dict | None, str | None]:
        data, err = await self._get("/api/stats/daily", {"days": days})
        return (data if isinstance(data, dict) else None), err

    async def get_stake_stats(self) -> tuple[dict | None, str | None]:
        data, err = await self._get("/api/stake/stats")
        return (data if isinstance(data, dict) else None), err

    async def get_markets(self) -> tuple[list | None, str | None]:
        data, err = await self._get("/api/markets")
        if isinstance(data, list):
            return data, err
        if isinstance(data, dict):
            rows = data.get("markets") or data.get("items") or data.get("data")
            if isinstance(rows, list):
                return rows, err
        return None, err or "bad shape"

    async def get_markets_feed(self, limit: int = 20) -> tuple[dict | None, str | None]:
        data, err = await self._get("/api/markets/feed", {"limit": limit})
        return (data if isinstance(data, dict) else None), err

    async def get_holders(self, mint: str | None = None) -> tuple[dict | None, str | None]:
        """$ANSEM holder snapshot from Jupiter + GeckoTerminal (+ Rugcheck tops)."""
        mint = (mint or ANSEM_MINT).strip()
        jup_url = f"https://lite-api.jup.ag/tokens/v2/search?query={mint}"
        gecko_url = f"https://api.geckoterminal.com/api/v2/networks/solana/tokens/{mint}/info"
        rug_url = f"https://api.rugcheck.xyz/v1/tokens/{mint}/report"

        (jup_raw, jup_err), (gecko_raw, gecko_err), (rug_raw, rug_err) = await asyncio.gather(
            self._get(jup_url, absolute=True),
            self._get(gecko_url, absolute=True),
            self._get(rug_url, absolute=True),
        )

        jup: dict[str, Any] | None = None
        if isinstance(jup_raw, list) and jup_raw:
            for row in jup_raw:
                if isinstance(row, dict) and row.get("id") == mint:
                    jup = row
                    break
            if jup is None and isinstance(jup_raw[0], dict):
                jup = jup_raw[0]
        elif isinstance(jup_raw, dict):
            jup = jup_raw

        gecko_attrs: dict[str, Any] = {}
        if isinstance(gecko_raw, dict):
            data = gecko_raw.get("data") or {}
            if isinstance(data, dict):
                gecko_attrs = data.get("attributes") or {}

        holders_block = gecko_attrs.get("holders") if isinstance(gecko_attrs.get("holders"), dict) else {}
        dist = holders_block.get("distribution_percentage") or {}

        top_holders: list[dict[str, Any]] = []
        if isinstance(rug_raw, dict):
            for h in (rug_raw.get("topHolders") or [])[:8]:
                if not isinstance(h, dict):
                    continue
                top_holders.append(
                    {
                        "owner": h.get("owner") or h.get("address"),
                        "address": h.get("address"),
                        "pct": h.get("pct"),
                        "ui_amount": h.get("uiAmount"),
                        "insider": h.get("insider"),
                    }
                )

        stats5m = (jup or {}).get("stats5m") or {}
        stats1h = (jup or {}).get("stats1h") or {}
        stats6h = (jup or {}).get("stats6h") or {}
        stats24h = (jup or {}).get("stats24h") or {}

        holder_count = (jup or {}).get("holderCount")
        if holder_count is None:
            holder_count = holders_block.get("count")
        if holder_count is None and isinstance(rug_raw, dict):
            holder_count = rug_raw.get("totalHolders")

        def _f(x: Any) -> float | None:
            if x is None or x == "":
                return None
            try:
                return float(x)
            except (TypeError, ValueError):
                return None

        out: dict[str, Any] = {
            "mint": mint,
            "holder_count": holder_count,
            "circ_supply": (jup or {}).get("circSupply") or gecko_attrs.get("normalized_total_supply"),
            "total_supply": (jup or {}).get("totalSupply"),
            "holder_change_5m": _f(stats5m.get("holderChange")),
            "holder_change_1h": _f(stats1h.get("holderChange")),
            "holder_change_6h": _f(stats6h.get("holderChange")),
            "holder_change_24h": _f(stats24h.get("holderChange")),
            "traders_1h": stats1h.get("numTraders"),
            "traders_24h": stats24h.get("numTraders"),
            "organic_buyers_24h": stats24h.get("numOrganicBuyers"),
            "net_buyers_24h": stats24h.get("numNetBuyers"),
            "buys_24h": stats24h.get("numBuys"),
            "sells_24h": stats24h.get("numSells"),
            "top10_pct": _f(dist.get("top_10")),
            "top11_20_pct": _f(dist.get("11_20")),
            "top21_40_pct": _f(dist.get("21_40")),
            "rest_pct": _f(dist.get("rest")),
            "dev_holding_pct": _f(gecko_attrs.get("developer_holding_percentage")),
            "mint_authority": gecko_attrs.get("mint_authority"),
            "freeze_authority": gecko_attrs.get("freeze_authority"),
            "top_holders": top_holders,
            "rug_score": (rug_raw or {}).get("score_normalised") if isinstance(rug_raw, dict) else None,
            "lp_locked_pct": (rug_raw or {}).get("lpLockedPct") if isinstance(rug_raw, dict) else None,
            "sources": {
                "jupiter": jup_err is None and jup is not None,
                "gecko": gecko_err is None and bool(gecko_attrs),
                "rugcheck": rug_err is None and isinstance(rug_raw, dict),
            },
            "errors": {
                k: v
                for k, v in {
                    "jupiter": jup_err,
                    "gecko": gecko_err,
                    "rugcheck": rug_err,
                }.items()
                if v
            },
        }

        if holder_count is None and not any(out["sources"].values()):
            return None, jup_err or gecko_err or rug_err or "holders unavailable"
        return out, None

    async def fetch_all(self) -> BoardSnapshot:
        keys = (
            "network",
            "config",
            "price",
            "ohlc",
            "stats",
            "daily",
            "stake",
            "markets",
            "feed",
            "holders",
        )
        coros = (
            self.get_network(),
            self.get_config(),
            self.get_token_price(),
            self.get_token_ohlc(),
            self.get_stats(),
            self.get_stats_daily(),
            self.get_stake_stats(),
            self.get_markets(),
            self.get_markets_feed(),
            self.get_holders(),
        )
        results = await asyncio.gather(*coros)
        errors: dict[str, str] = {}
        payload: dict[str, Any] = {}
        for key, (data, err) in zip(keys, results, strict=True):
            payload[key] = data
            if err:
                errors[key] = err
        return BoardSnapshot(
            network=payload.get("network"),
            config=payload.get("config"),
            price=payload.get("price"),
            ohlc=payload.get("ohlc"),
            stats=payload.get("stats"),
            daily=payload.get("daily"),
            stake=payload.get("stake"),
            markets=payload.get("markets"),
            feed=payload.get("feed"),
            holders=payload.get("holders"),
            errors=errors,
            fetched_at=_now_iso(),
        )


async def fetch_snapshot(base: str | None = None) -> BoardSnapshot:
    return await BullClient(base).fetch_all()
