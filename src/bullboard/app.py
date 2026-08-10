"""Textual multi-pane Surfboard-style dashboard."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from textual import work
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.widgets import Static

from bullboard import __version__
from bullboard.client import BullClient
from bullboard.config import (
    REFRESH_FAST_SEC,
    REFRESH_FEED_SEC,
    SECOND_X_HANDLE,
    X_HANDLE,
)
from bullboard.formatters import (
    ago,
    clock_mmdd_hhmm,
    delta_str,
    fmt_ansem,
    fmt_compact,
    fmt_usd,
    short_addr,
    sparkline,
)
from bullboard.models import BoardSnapshot, signals_from_snapshot
from bullboard.xfeed import fetch_announce_feed

_FEED_MODES = ("primary", "alt", "both")
_TCSS = Path(__file__).with_name("theme.tcss")


def _lines(body: str) -> str:
    return body.rstrip() + "\n"


class Panel(Vertical):
    DEFAULT_CSS = """
    Panel {
        border: solid #2a2e24;
        background: #121410;
        padding: 0 1;
        height: 1fr;
    }
    Panel > .panel-title {
        color: #c8f542;
        text-style: bold;
        height: 1;
    }
    Panel > .panel-body {
        height: 1fr;
        color: #c8d0b8;
    }
    """

    def __init__(self, title: str, body_id: str, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self._title = title
        self._body_id = body_id

    def compose(self) -> ComposeResult:
        yield Static(self._title, classes="panel-title")
        yield Static("…", id=self._body_id, classes="panel-body")


class BullboardApp(App[None]):
    CSS_PATH = _TCSS
    TITLE = "bullboard"
    BINDINGS = [
        Binding("q", "quit", "quit", show=True),
        Binding("r", "refresh", "refresh", show=True),
        Binding("n", "cycle_feed", "feed", show=True),
    ]

    def __init__(self, api_base: str | None = None) -> None:
        super().__init__()
        self.client = BullClient(api_base)
        self.snap = BoardSnapshot()
        self.feed_mode = "primary"
        self._last_fetch: str | None = None
        self._loading = False

    def compose(self) -> ComposeResult:
        yield Static(self._header_text(), id="header")
        with Horizontal(id="top-row"):
            yield Panel("BULL GATE", "gate-body")
            yield Panel("TREASURY / MINT", "treasury-body")
            yield Panel("STAKE / FEES", "stake-body")
            yield Panel("ANSEM MCAP", "mcap-body")
        with Horizontal(id="mid-row"):
            yield Panel(self._announce_title(), "announce-body", id="announce")
            with Vertical(id="mid-right"):
                yield Panel("SIGNALS", "signals-body", id="signals")
                yield Panel("INFERENCE ACTIVITY", "activity-body", id="activity")
        with Horizontal(id="bottom-row"):
            yield Panel("ANSEM MARKET", "market-body")
            yield Panel("BULL.INF DESK", "desk-body")
        yield Static(self._footer_text(), id="footer")

    def on_mount(self) -> None:
        self.set_interval(REFRESH_FAST_SEC, self._tick_data)
        self.set_interval(REFRESH_FEED_SEC, self._tick_feed)
        self.refresh_all()

    def _header_text(self) -> str:
        ch = self.snap.change_24h()
        parity = delta_str(ch) if ch is not None else "parity —"
        handle = self._active_handle_label()
        act = ((self.snap.stats or {}).get("activity_24h") or {}).get("requests")
        act_s = f"feed · {act or 0} req/24h" if act is not None else "feed · activity"
        return f" BULLBOARD · $ANSEM · @{handle} · {parity} · {act_s} "

    def _footer_text(self) -> str:
        updated = ago(self._last_fetch) if self._last_fetch else "never"
        mode = self.feed_mode
        return (
            f" q quit · r refresh · n cycle feed ({mode}) · "
            f"updated {updated} · bullboard v{__version__} "
        )

    def _active_handle_label(self) -> str:
        if self.feed_mode == "alt":
            return SECOND_X_HANDLE
        if self.feed_mode == "both":
            return f"{X_HANDLE}+{SECOND_X_HANDLE}"
        return X_HANDLE

    def _announce_title(self) -> str:
        last = "—"
        tweets = (self.snap.tweets or {}).get("tweets") or []
        if tweets:
            last = ago(tweets[0].get("created_at"))
        return f"ANNOUNCE FEED · @{self._active_handle_label()} · last {last}"

    def action_refresh(self) -> None:
        self.refresh_all(force_feed=True)

    def action_cycle_feed(self) -> None:
        i = _FEED_MODES.index(self.feed_mode) if self.feed_mode in _FEED_MODES else 0
        self.feed_mode = _FEED_MODES[(i + 1) % len(_FEED_MODES)]
        self.query_one("#announce .panel-title", Static).update(self._announce_title())
        self.query_one("#footer", Static).update(self._footer_text())
        self.refresh_feed()

    def _tick_data(self) -> None:
        self.refresh_data()

    def _tick_feed(self) -> None:
        self.refresh_feed()

    def refresh_all(self, force_feed: bool = False) -> None:
        self.refresh_data()
        self.refresh_feed(force=force_feed)

    @work(exclusive=True, group="data")
    async def refresh_data(self) -> None:
        snap = await self.client.fetch_all()
        # preserve tweets / feed_mode
        snap.tweets = self.snap.tweets
        snap.feed_mode = self.feed_mode
        self.snap = snap
        self._last_fetch = snap.fetched_at
        self._paint()

    @work(exclusive=True, group="feed")
    async def refresh_feed(self, force: bool = False) -> None:
        tweets = await fetch_announce_feed(self.feed_mode, limit=24)
        self.snap.tweets = tweets
        self.snap.feed_mode = self.feed_mode
        self._paint_announce()
        self.query_one("#header", Static).update(self._header_text())
        self.query_one("#footer", Static).update(self._footer_text())

    def _paint(self) -> None:
        s = self.snap
        net = s.network or {}
        cfg = s.config or {}
        stake = s.stake or {}
        stats = s.stats or {}
        ohlc_stats = (s.ohlc or {}).get("stats") or {}
        price = s.price_usd()
        ch = s.change_24h()

        # GATE
        min_w = net.get("min_wallet_ansem", cfg.get("min_wallet_ansem"))
        onchain = net.get("per_call_onchain", cfg.get("per_call_onchain"))
        ready = net.get("solana_ready")
        cluster = net.get("cluster") or "—"
        gate = (
            f"min wallet   {min_w if min_w is not None else '—'} $ANSEM\n"
            f"per-call     {'ON' if onchain else 'off'}\n"
            f"solana       {'READY' if ready else 'DOWN'}\n"
            f"cluster      {cluster}\n"
            f"mode         {net.get('transfer_mode') or '—'}"
        )
        self.query_one("#gate-body", Static).update(gate)

        # TREASURY
        treasury = net.get("treasury") or net.get("settlement_authority")
        mint = net.get("mint") or "—"
        treas = (
            f"treasury  {short_addr(treasury, 6)}\n"
            f"mint      {short_addr(mint, 6)}\n"
            f"token     {net.get('token_name') or 'The Black Bull'}\n"
            f"symbol    {net.get('token_symbol') or '$ANSEM'}\n"
            f"decimals  {net.get('decimals') if net.get('decimals') is not None else '—'}"
        )
        self.query_one("#treasury-body", Static).update(treas)

        # STAKE
        fee = stake.get("platform_fee", cfg.get("platform_fee"))
        staker = stake.get("staker_fee_rate", cfg.get("staker_fee_rate"))
        buyback = stake.get("buyback_fee_rate", cfg.get("buyback_fee_rate"))
        routed = stake.get("fees_routed_24h_ansem")
        pool = stake.get("pool_pending_ansem")

        def _rate(x: Any) -> str:
            if x is None:
                return "—"
            try:
                v = float(x)
            except (TypeError, ValueError):
                return "—"
            if abs(v) <= 1:
                v *= 100
            return f"{v:.1f}%"

        stake_body = (
            f"platform   {_rate(fee)}\n"
            f"staker     {_rate(staker)}\n"
            f"buyback    {_rate(buyback)}\n"
            f"routed 24h {fmt_ansem(routed)}\n"
            f"pool       {fmt_ansem(pool)}"
        )
        self.query_one("#stake-body", Static).update(stake_body)

        # MCAP
        mcap = ohlc_stats.get("market_cap") or ohlc_stats.get("fdv")
        fdv = ohlc_stats.get("fdv")
        mcap_body = (
            f"price   {fmt_usd(price)}\n"
            f"24h     {delta_str(ch)}\n"
            f"mcap    {fmt_usd(mcap, digits=2)}\n"
            f"fdv     {fmt_usd(fdv, digits=2)}\n"
            f"src     {(s.price or {}).get('source') or ohlc_stats.get('source') or '—'}"
        )
        self.query_one("#mcap-body", Static).update(mcap_body)

        # SIGNALS
        sig_lines = []
        for status, label, detail in signals_from_snapshot(s):
            mark = {"ok": "●", "warn": "◐", "bad": "○"}.get(status, "·")
            sig_lines.append(f"{mark} {label:<12} {detail}")
        self.query_one("#signals-body", Static).update("\n".join(sig_lines) or "—")

        # ACTIVITY
        feed_items = (s.feed or {}).get("feed") or []
        act_lines = []
        for ev in feed_items[:12]:
            t = clock_mmdd_hhmm(ev.get("created_at"))
            model = (ev.get("model_id") or "?")[:22]
            cost = ev.get("cost")
            act_lines.append(f"{t}  {model:<22} {fmt_ansem(cost)}")
        self.query_one("#activity-body", Static).update("\n".join(act_lines) or "no inference yet")

        # MARKET
        vol = ohlc_stats.get("volume_24h")
        liq = ohlc_stats.get("liquidity_usd")
        spark = sparkline(s.closes(), width=28)
        market = (
            f"price  {fmt_usd(price)}   24h {delta_str(ch)}\n"
            f"vol    {fmt_usd(vol, digits=2)}   liq {fmt_usd(liq, digits=2)}\n"
            f"hi/lo  {fmt_usd(ohlc_stats.get('high_24h'))} / {fmt_usd(ohlc_stats.get('low_24h'))}\n"
            f"{spark}\n"
            f"pair   {short_addr(ohlc_stats.get('pair_address'), 6)}"
        )
        self.query_one("#market-body", Static).update(market)

        # DESK
        act24 = stats.get("activity_24h") or {}
        desk = (
            f"requests     {fmt_compact(stats.get('requests'))}  (24h {fmt_compact(act24.get('requests'))})\n"
            f"sellers      {fmt_compact(stats.get('active_sellers'))} active / {fmt_compact(stats.get('registered_sellers'))} reg\n"
            f"models live  {fmt_compact(stats.get('live_models'))}   offers {fmt_compact(stats.get('open_offers'))}\n"
            f"liquidity    {fmt_compact(stats.get('liquidity_remaining'))}\n"
            f"gross        {fmt_ansem(stats.get('gross_revenue'))}  · savings ${fmt_compact(stats.get('savings_usd'))}"
        )
        self.query_one("#desk-body", Static).update(desk)

        self._paint_announce()
        self.query_one("#header", Static).update(self._header_text())
        self.query_one("#footer", Static).update(self._footer_text())

    def _paint_announce(self) -> None:
        try:
            self.query_one("#announce .panel-title", Static).update(self._announce_title())
        except Exception:  # noqa: BLE001
            pass
        tweets = (self.snap.tweets or {}).get("tweets") or []
        err = (self.snap.tweets or {}).get("error")
        lines: list[str] = []
        for t in tweets[:18]:
            when = clock_mmdd_hhmm(t.get("created_at"))
            handle = t.get("handle")
            prefix = f"@{handle} " if handle and self.feed_mode == "both" else ""
            text = (t.get("text") or "").replace("\n", " ")
            if len(text) > 110:
                text = text[:107] + "…"
            lines.append(f"{when} POST  {prefix}{text}")
        if not lines:
            msg = "no posts · feed offline" if err else "no posts in window"
            if err:
                msg += f"\n{err[:80]}"
            lines = [msg]
        self.query_one("#announce-body", Static).update("\n".join(lines))


def run_app(api_base: str | None = None) -> None:
    BullboardApp(api_base=api_base).run()
