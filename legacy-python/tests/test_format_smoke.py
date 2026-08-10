"""Smoke tests for formatters + client --once path."""

from __future__ import annotations

from bullboard.formatters import delta_str, fmt_compact, fmt_usd, sparkline


def test_fmt_usd():
    assert fmt_usd(0.1927).startswith("$0.19")
    assert fmt_usd(1_500_000) == "$1.50M"
    assert fmt_usd(None) == "—"


def test_sparkline():
    s = sparkline([1, 2, 3, 2, 5], width=5)
    assert len(s) == 5
    assert sparkline([]) 


def test_delta():
    assert "▲" in delta_str(1.2)
    assert "▼" in delta_str(-3.4)


def test_compact():
    assert fmt_compact(12_500) == "12.50K"
