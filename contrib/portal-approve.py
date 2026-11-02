#!/usr/bin/env python3
"""Auto-approve GNOME portal dialogs for `spacing` (Capture Input and
Remote Desktop) via AT-SPI — no manual clicking needed.

GNOME's portal backend (xdg-desktop-portal-gnome >= 45) shows a GTK
permission dialog for both portals:

  * serve (InputCapture CreateSession): "An application wants to
    capture input events" + "Allow Remote Input Capturing" switch
  * client (RemoteDesktop Start): "Allow Remote Interaction" switch

The request only resolves after the switch is toggled on and "Share"
is clicked. This script walks the AT-SPI accessibility tree of
xdg-desktop-portal-gnome, toggles every "Allow …" switch, then clicks
"Share" — repeating until the dialog is gone or `--timeout` elapses.

Usage:
    python3 contrib/portal-approve.py [--timeout 60]

Run it in a second terminal while `spacing serve` / `spacing client`
is waiting on the dialog, or leave it looping in the background for a
hands-free smoke test. Requires: dbus-python, an AT-SPI bus
(org.a11y.Bus — present on any standard GNOME session).
"""
import argparse
import sys
import time

import dbus

ACC = "org.a11y.atspi.Accessible"
ACTION = "org.a11y.atspi.Action"
PROPS = "org.freedesktop.DBus.Properties"
ROOT = "/org/a11y/atspi/accessible/root"

ROLE_PUSH_BUTTON = 43
ROLE_DIALOG = 23
ROLE_SWITCH = 130  # GTK4 exposes GtkSwitch/AdwSwitchRow under this role

DIALOG_TITLES = ("remote desktop", "capture input")
SWITCH_HINTS = ("allow remote input capturing", "allow remote interaction",
                "allow remote")
BUTTON_LABELS = ("share", "allow", "accept", "connect")


def a11y_bus():
    sb = dbus.SessionBus()
    addr = sb.get_object("org.a11y.Bus", "/org/a11y/bus",
                         introspect=False).GetAddress(
                             dbus_interface="org.a11y.Bus")
    return dbus.bus.BusConnection(addr)


def acc(bus, name, path):
    return bus.get_object(name, path, introspect=False)


def name_of(o):
    try:
        return str(o.Get(ACC, "Name", dbus_interface=PROPS))
    except Exception:
        return ""


def role_of(o):
    try:
        return int(o.GetRole(dbus_interface=ACC))
    except Exception:
        return -1


def actions_of(o):
    try:
        return [str(a[0]) for a in o.GetActions(dbus_interface=ACTION)]
    except Exception:
        return []


def do(o, verb):
    acts = actions_of(o)
    for i, a in enumerate(acts):
        if a.lower() == verb:
            o.DoAction(dbus.Int32(i), dbus_interface=ACTION)
            return True
    return False


def walk(bus, name, pred, path=ROOT, depth=0, out=None):
    if out is None:
        out = []
    if depth > 15:
        return out
    o = acc(bus, name, path)
    try:
        if pred(o):
            out.append(o)
    except Exception:
        return out
    try:
        children = o.GetChildren(dbus_interface=ACC)
    except Exception:
        return out
    for ch in children:
        walk(bus, name, pred, str(ch[1]), depth + 1, out)
    return out


def portal_app(bus):
    for n in bus.list_names():
        if not str(n).startswith(":"):
            continue
        try:
            root = acc(bus, n, ROOT)
            if name_of(root) == "xdg-desktop-portal-gnome":
                return n
        except Exception:
            continue
    return None


def approve_once(bus, verbose):
    """Returns True if a dialog was found and handled this pass."""
    app = portal_app(bus)
    if not app:
        return False
    dialogs = walk(bus, app, lambda o: role_of(o) == ROLE_DIALOG
                   and name_of(o).lower() in DIALOG_TITLES)
    if not dialogs:
        return False
    title = name_of(dialogs[0])
    if verbose:
        print(f'dialog: "{title}"', flush=True)

    # Toggle on every "Allow …" switch.
    switches = walk(
        bus, app,
        lambda o: role_of(o) == ROLE_SWITCH and "Toggle" in actions_of(o))
    for s in switches:
        if any(h in name_of(s).lower() for h in SWITCH_HINTS):
            do(s, "Toggle")
            if verbose:
                print(f'  toggle "{name_of(s)}"', flush=True)
    time.sleep(0.5)  # buttons may appear only after toggling

    # Click the affirmative button.
    for label in BUTTON_LABELS:
        btns = walk(bus, app, lambda o, L=label:
                    role_of(o) == ROLE_PUSH_BUTTON
                    and name_of(o).lower() == L
                    and "Click" in actions_of(o))
        if btns:
            do(btns[0], "Click")
            if verbose:
                print(f'  click "{name_of(btns[0])}"', flush=True)
            return True
    return True


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--timeout", type=float, default=60.0,
                    help="seconds to keep watching (default 60)")
    ap.add_argument("--once", action="store_true",
                    help="handle one dialog then exit")
    ap.add_argument("-q", "--quiet", action="store_true")
    args = ap.parse_args()
    verbose = not args.quiet

    bus = a11y_bus()
    deadline = time.time() + args.timeout
    seen = False
    while time.time() < deadline:
        if approve_once(bus, verbose):
            seen = True
            if args.once:
                break
            time.sleep(0.5)
        else:
            if seen:
                break  # dialog handled and dismissed
            time.sleep(0.5)
    if verbose and not seen:
        print("no portal dialog seen before timeout", flush=True)
    return 0 if seen else 1


if __name__ == "__main__":
    sys.exit(main())
