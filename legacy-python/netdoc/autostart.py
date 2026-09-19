"""Start NetDoctor with Windows.

Uses the per-user Run key, so it needs no admin rights. The monitor itself does
not need elevation — only applying tweaks does — so an unelevated autostart is
enough to keep catching dropouts.
"""

import os
import sys
from pathlib import Path

from . import probes

RUN_KEY = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run"
VALUE_NAME = "NetDoctor"


def _pythonw():
    """pythonw.exe next to the running interpreter, so no console window."""
    exe = Path(sys.executable)
    candidate = exe.with_name("pythonw.exe")
    return str(candidate if candidate.exists() else exe)


def _script():
    return str(Path(__file__).resolve().parent.parent / "run.py")


def command_line():
    return '"%s" "%s" --minimised' % (_pythonw(), _script())


def is_enabled():
    rc, out = probes.run(["reg", "query", RUN_KEY, "/v", VALUE_NAME], timeout=15)
    return rc == 0 and VALUE_NAME in out


def current_value():
    rc, out = probes.run(["reg", "query", RUN_KEY, "/v", VALUE_NAME], timeout=15)
    for line in out.splitlines():
        if VALUE_NAME in line and "REG_SZ" in line:
            return line.split("REG_SZ", 1)[1].strip()
    return ""


def enable():
    if not os.path.exists(_script()):
        return False, "Nie znaleziono run.py — uruchom aplikację z jej katalogu."
    rc, out = probes.run(
        ["reg", "add", RUN_KEY, "/v", VALUE_NAME, "/t", "REG_SZ",
         "/d", command_line(), "/f"], timeout=15)
    if rc != 0:
        return False, out.strip()[:300] or "zapis do rejestru nie powiódł się"
    return True, "NetDoctor będzie startował zminimalizowany przy logowaniu."


def disable():
    rc, out = probes.run(["reg", "delete", RUN_KEY, "/v", VALUE_NAME, "/f"], timeout=15)
    if rc != 0 and "unable to find" not in out.lower():
        return False, out.strip()[:300] or "usunięcie wpisu nie powiodło się"
    return True, "Autostart wyłączony."


def toggle(on):
    return enable() if on else disable()
