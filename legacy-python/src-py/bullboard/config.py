"""Runtime config + constants."""

from __future__ import annotations

import os

API_BASE = (os.environ.get("BULLBOARD_API_BASE") or "https://api.bullinf.fun").rstrip("/")
X_HANDLE = (os.environ.get("BULLBOARD_X_HANDLE") or "blknoiz06").lstrip("@")
SECOND_X_HANDLE = (os.environ.get("BULLBOARD_X_HANDLE_ALT") or "bullinference").lstrip("@")

ANSEM_MINT = "9cRCn9rGT8V2imeM2BaKs13yhMEais3ruM3rPvTGpump"
ANSEM_SYMBOL = "$ANSEM"
TOKEN_NAME = "The Black Bull"

REFRESH_FAST_SEC = float(os.environ.get("BULLBOARD_REFRESH_SEC") or "15")
REFRESH_FEED_SEC = float(os.environ.get("BULLBOARD_FEED_REFRESH_SEC") or "60")

NITTER_BASES = [
    b.strip().rstrip("/")
    for b in (
        os.environ.get("BULLBOARD_NITTER_BASES")
        or "https://nitter.net,https://nitter.privacydev.net,https://nitter.poast.org"
    ).split(",")
    if b.strip()
]

# Brand (Bull desk)
INK = "#0a0b09"
ACID = "#c8f542"
MUTED = "#6b6f64"
PANEL = "#121410"
BORDER = "#2a2e24"
