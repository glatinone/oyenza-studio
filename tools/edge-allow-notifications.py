"""Allow a site's notifications in Edge (last used profile), so its toasts reach Floaty.

    python tools/edge-allow-notifications.py https://web.whatsapp.com [--jx PATH]

Already allowed: nothing to do. Edge closed: the exception is written into the
profile's Preferences. Edge open: Edge would overwrite that file, so the real
permission prompt is raised on a same-origin page through jx (Kiell's Edge) and
its Allow button is pressed with UI Automation. Exit 0 = allowed.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
from urllib.parse import urlsplit

USER_DATA = os.path.expandvars(r"%LOCALAPPDATA%\Microsoft\Edge\User Data")

# Presses Allow on the "<host> wants to: Show notifications" bubble (the button
# only, never a keystroke into whatever has focus).
UIA = r"""
param([string]$Ask)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$wins = New-Object System.Windows.Automation.PropertyCondition($A::ClassNameProperty, 'Chrome_WidgetWin_1')
$btn = New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
for ($i = 0; $i -lt 40; $i++) {
  Start-Sleep -Milliseconds 250
  foreach ($w in $A::RootElement.FindAll('Children', $wins)) {
    $q = $w.FindFirst('Descendants', (New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, $Ask)))
    if (-not $q) { continue }
    $b = $w.FindAll('Descendants', $btn) | Where-Object { $_.Current.Name -eq 'Allow' } | Select-Object -First 1
    if ($b) { $b.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); 'clicked'; exit 0 }
  }
}
'no prompt'; exit 1
"""


def profile_dir():
    try:
        with open(os.path.join(USER_DATA, "Local State"), encoding="utf-8") as f:
            last = json.load(f)["profile"].get("last_used") or "Default"
    except (OSError, ValueError, KeyError):
        last = "Default"
    return os.path.join(USER_DATA, last)


def pattern(origin):
    u = urlsplit(origin)
    port = u.port or (443 if u.scheme == "https" else 80)
    return f"{u.scheme}://{u.hostname}:{port},*"


def read_prefs(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def allowed(prefs, key):
    n = prefs.get("profile", {}).get("content_settings", {}).get("exceptions", {}).get("notifications", {})
    return n.get(key, {}).get("setting") == 1


def edge_running():
    out = subprocess.run(["tasklist", "/FI", "IMAGENAME eq msedge.exe", "/NH"], capture_output=True, text=True).stdout
    return "msedge.exe" in out


def write_prefs(path, prefs, key):
    # Chrome/Edge time: microseconds since 1601-01-01.
    now = str(int((time.time() + 11644473600) * 1_000_000))
    ex = prefs.setdefault("profile", {}).setdefault("content_settings", {}).setdefault("exceptions", {})
    ex.setdefault("notifications", {})[key] = {"last_modified": now, "setting": 1}
    tmp = path + ".floaty-tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(prefs, f, separators=(",", ":"), ensure_ascii=False)
    os.replace(tmp, path)


def via_jx(origin, jx_dir):
    sys.path.insert(0, jx_dir)
    import jx  # noqa: E402  (Kiell's jx checkout)

    host = urlsplit(origin).hostname
    ps1 = os.path.join(tempfile.gettempdir(), "floaty-allow-notifications.ps1")
    with open(ps1, "w", encoding="utf-8") as f:
        f.write(UIA)
    result = {}

    def press():
        r = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ps1,
                            "-Ask", f"{host} wants to: Show notifications"], capture_output=True, text=True)
        result["uia"] = r.stdout.strip()

    def value(r):
        return (r.get("steps") or [{}])[-1].get("value")

    with jx.Session(real=True) as s:
        s(f"goto {origin}/favicon.ico")
        if value(s("eval Notification.permission")) == "granted":
            return True
        s('eval document.body.innerHTML="<button id=n style=\'font-size:40px\'>Allow notifications</button>";'
          ' document.getElementById("n").onclick=()=>Notification.requestPermission(); "ok"')
        t = threading.Thread(target=press)
        t.start()
        s('click "#n"')
        t.join(15)
        time.sleep(0.8)
        return value(s("eval Notification.permission")) == "granted"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("origin")
    ap.add_argument("--jx", default=os.path.join(os.path.expanduser("~"), "Videos", "Tools", "jx"))
    a = ap.parse_args()
    origin = a.origin.rstrip("/")
    prefs_path = os.path.join(profile_dir(), "Preferences")
    key = pattern(origin)

    if os.path.exists(prefs_path) and allowed(read_prefs(prefs_path), key):
        print(f"{origin}: notifications already allowed")
        return 0
    if not edge_running() and os.path.exists(prefs_path):
        write_prefs(prefs_path, read_prefs(prefs_path), key)
        print(f"{origin}: notifications allowed (Edge profile updated while closed)")
        return 0
    if os.path.exists(os.path.join(a.jx, "jx.py")):
        if via_jx(origin, a.jx):
            print(f"{origin}: notifications allowed (Edge prompt answered via jx)")
            return 0
        print(f"{origin}: the Edge prompt did not come up; retried on the next setup run")
        return 1
    print(f"{origin}: Edge is open and jx is missing; allowed on the next setup run with Edge closed")
    return 1


if __name__ == "__main__":
    sys.exit(main())
