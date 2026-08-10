"""Async Bull.inf public API client."""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone
from typing import Any

import httpx

from bullboard.config import API_BASE
from bullboard.models import BoardSnapshot

_TIMEOUT = httpx.Timeout(12.0, connect=8.0)


def _now_iso() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


class BullClient:
    def __init__(self, base: str | None = None) -> None:
        self.base = (base or API_BASE).rstrip("/")

    async def _get(self, path: str, params: dict[str, Any] | None = None) -> tuple[Any | None, str | None]:
        url = f"{self.base}{path}"
        try:
            async with httpx.AsyncClient(timeout=_TIMEOUT, headers={"User-Agent": "bullboard/0.1"}) as client:
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
            errors=errors,
            fetched_at=_now_iso(),
        )


async def fetch_snapshot(base: str | None = None) -> BoardSnapshot:
    return await BullClient(base).fetch_all()
