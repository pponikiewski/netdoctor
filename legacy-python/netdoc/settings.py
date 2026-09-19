"""User settings, persisted as JSON.

config.py holds the defaults; this module holds what the user changed. Anything
missing from the file falls back to the default, so an older settings file keeps
working after an update.
"""

import json
import threading

from . import config

SETTINGS_PATH = config.DATA_DIR / "settings.json"

DEFAULTS = {
    # probing
    "probe_interval_s": config.PROBE_INTERVAL_S,
    "ping_timeout_ms": config.PING_TIMEOUT_MS,
    "outage_after_fails": config.OUTAGE_AFTER_FAILS,
    "history_points": config.HISTORY_POINTS,
    "extra_targets": [],              # user-added hosts, e.g. a game server
    # thresholds
    "ping_good": config.PING_GOOD,
    "ping_ok": config.PING_OK,
    "ping_bad": config.PING_BAD,
    "jitter_good": config.JITTER_GOOD,
    "jitter_ok": config.JITTER_OK,
    "loss_good": config.LOSS_GOOD,
    "loss_ok": config.LOSS_OK,
    # behaviour
    "notify_on_outage": True,
    "start_minimised": False,
    "keep_days": 14,
}


class Settings:
    """Attribute access over a JSON-backed dict."""

    def __init__(self, path=SETTINGS_PATH):
        self._path = path
        self._lock = threading.Lock()
        self._data = dict(DEFAULTS)
        self.load()

    def load(self):
        try:
            with open(self._path, "r", encoding="utf-8") as f:
                stored = json.load(f)
            if isinstance(stored, dict):
                for k, v in stored.items():
                    if k in DEFAULTS:
                        self._data[k] = v
        except (OSError, ValueError):
            pass
        return self

    def save(self):
        with self._lock:
            self._path.parent.mkdir(parents=True, exist_ok=True)
            with open(self._path, "w", encoding="utf-8") as f:
                json.dump(self._data, f, indent=2, ensure_ascii=False)

    def reset(self):
        self._data = dict(DEFAULTS)
        self.save()

    def __getattr__(self, name):
        try:
            return self.__dict__["_data"][name]
        except KeyError:
            raise AttributeError(name) from None

    def __setattr__(self, name, value):
        if name.startswith("_"):
            super().__setattr__(name, value)
        elif name in DEFAULTS:
            self._data[name] = value
        else:
            super().__setattr__(name, value)

    def as_dict(self):
        return dict(self._data)

    def update(self, **kw):
        for k, v in kw.items():
            if k in DEFAULTS:
                self._data[k] = v
        self.save()

    def targets(self):
        """Built-in targets plus whatever the user added."""
        out = list(config.TARGETS)
        for i, host in enumerate(self.extra_targets):
            host = str(host).strip()
            if host:
                out.append({"key": "custom%d" % i, "label": host,
                            "host": host, "scope": "internet"})
        return out


S = Settings()
