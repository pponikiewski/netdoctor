"""Static configuration: probe targets, thresholds, file locations."""

import os
from pathlib import Path

APP_NAME = "NetDoctor"

DATA_DIR = Path(os.environ.get("LOCALAPPDATA", Path.home())) / APP_NAME
DB_PATH = DATA_DIR / "history.db"
SNAPSHOT_PATH = DATA_DIR / "tweak_snapshots.json"

# Probe targets. "gateway" is resolved at runtime from the routing table.
TARGETS = [
    {"key": "gateway", "label": "Router (brama)", "host": None, "scope": "lan"},
    {"key": "dns_isp", "label": "DNS operatora", "host": None, "scope": "isp"},
    {"key": "cloudflare", "label": "Cloudflare 1.1.1.1", "host": "1.1.1.1", "scope": "internet"},
    {"key": "google", "label": "Google 8.8.8.8", "host": "8.8.8.8", "scope": "internet"},
]

# Hostname used to time DNS resolution (cache-busted per query).
DNS_TEST_DOMAIN = "example.com"

PROBE_INTERVAL_S = 1.0        # one sweep per second
PING_TIMEOUT_MS = 1000
OUTAGE_AFTER_FAILS = 3        # consecutive failed sweeps before declaring an outage
HISTORY_POINTS = 300          # live chart width in samples

# Quality thresholds (ms) used for colour coding and verdicts.
PING_GOOD = 30
PING_OK = 60
PING_BAD = 120
JITTER_GOOD = 5
JITTER_OK = 15
LOSS_GOOD = 0.5               # percent
LOSS_OK = 2.0
