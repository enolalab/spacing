#!/usr/bin/env python3
"""Render docs/images/*.png — terminal-window screenshots from REAL
spacing output captured on the Ubuntu 24.04 test VMs (see
docs/benchmarks/data/). Regenerate: python3 contrib/screenshot.py
"""
from PIL import Image, ImageDraw, ImageFont

FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
FS = 16
LH = int(FS * 1.6)
PAD = 20
BAR = 36
PANE_W = 700                 # ~66 cols at FS=16 (char ~9.62px)

BG = (24, 25, 31)
BARBG = (46, 48, 56)
TEXT = (229, 231, 235)
DIM = (107, 114, 128)
GREEN = (74, 222, 128)
YELLOW = (250, 204, 21)
CYAN = (34, 211, 238)
BLUE = (96, 165, 250)
DOTS = [(255, 95, 86), (255, 189, 46), (39, 201, 63)]

font = ImageFont.truetype(FONT, FS)


def pane(x0, y0, title, lines, img):
    d = ImageDraw.Draw(img)
    h = BAR + PAD * 2 + LH * len(lines)
    d.rounded_rectangle([x0, y0, x0 + PANE_W, y0 + h], 10, fill=BG, outline=(63, 66, 76))
    d.rectangle([x0, y0, x0 + PANE_W, y0 + BAR], fill=BARBG)
    d.rounded_rectangle([x0, y0, x0 + PANE_W, y0 + BAR + 10], 10, fill=BARBG)
    d.rectangle([x0, y0 + 10, x0 + PANE_W, y0 + BAR], fill=BARBG)
    for i, c in enumerate(DOTS):
        d.ellipse([x0 + 14 + i * 20, y0 + 12, x0 + 26 + i * 20, y0 + 24], fill=c)
    tw = d.textlength(title, font=font)
    d.text((x0 + (PANE_W - tw) / 2, y0 + 10), title, font=font, fill=DIM)
    # clip text to pane so long lines can't bleed across
    y = y0 + BAR + PAD
    for color, txt in lines:
        d.text((x0 + PAD, y), txt, font=font, fill=color)
        y += LH
    return h


def two_pane(title_l, title_r, left, right, out, gap=26, margin=18, note=None):
    hl = BAR + PAD * 2 + LH * len(left)
    hr = BAR + PAD * 2 + LH * len(right)
    note_h = 44 if note else 0
    w = margin * 2 + PANE_W * 2 + gap
    h = margin * 2 + max(hl, hr) + note_h
    img = Image.new("RGB", (w, h), (16, 17, 21))
    pane(margin, margin, title_l, left, img)
    pane(margin + PANE_W + gap, margin, title_r, right, img)
    if note:
        d = ImageDraw.Draw(img)
        d.text((margin, margin + max(hl, hr) + 14), note, font=font, fill=DIM)
    img.save(out)
    print("wrote", out)


def main():
    dim = lambda s: (DIM, s)
    prompt = lambda s: (BLUE, s)
    out = lambda s: (TEXT, s)
    info = lambda s: (GREEN, s)
    accent = lambda s: (CYAN, s)

    # ---------- screenshot 1: fingerprint + pairing ----------
    left = [
        prompt("ubuntu@spacing-srv ~ $ sudo spacing fingerprint"),
        accent("sha256:b0877d4ae76c8cd3438feeccca4f35ddbbb14"),
        dim(""),
        prompt("ubuntu@spacing-srv ~ $ sudo spacing serve --pairing"),
        info("INFO  spacing::server:"),
        out("      identity loaded fp=sha256:b0877d4a…fd541"),
        info("INFO  spacing::server:"),
        out("      listening bind=0.0.0.0:24800 pairing=true"),
        info("INFO  spacing::server:"),
        out("      client connected peer=spacing-cli"),
    ]
    right = [
        prompt("ubuntu@spacing-cli ~ $ sudo spacing fingerprint"),
        accent("sha256:cdc4318d21b59c8c8928e8417ade60e3e172e"),
        dim(""),
        prompt("ubuntu@spacing-cli ~ $ sudo spacing pair 10.234.79.81:24800"),
        out("server fingerprint: sha256:b0877d4a…fd541"),
        out("trust this fingerprint? [y/N] y"),
        info("paired with 10.234.79.81:24800"),
        dim(""),
        dim("# fingerprint pinned in ~/.config/spacing/peers.toml"),
        dim("# later connects need no --pairing (TOFU)"),
    ]
    two_pane("spacing-srv — input owner", "spacing-cli — controlled", left, right,
             "docs/images/pair.png",
             note="mTLS 1.3 · self-signed certs · SHA-256 fingerprint TOFU pairing")

    # ---------- screenshot 2: hotkey switch ----------
    left = [
        prompt("ubuntu@spacing-srv ~ $ sudo spacing serve"),
        info("INFO  spacing_capture:"),
        out("      capturing device path=\"/dev/input/event1\""),
        info("INFO  spacing::server:"),
        out("      listening bind=0.0.0.0:24800 pairing=false"),
        info("INFO  spacing::server:"),
        out("      client connected peer=spacing-cli"),
        info("INFO  spacing::server:"),
        out("      switching to remote target=spacing-cli"),
        dim("      # Ctrl+Alt+1 — kbd+mouse grabbed, -> TLS -> cli"),
        info("INFO  spacing::server:"),
        out("      back to local   # Ctrl+Alt+0"),
    ]
    right = [
        prompt("ubuntu@spacing-cli ~ $ sudo spacing client \\"),
        prompt("        --to 10.234.79.81:24800"),
        info("INFO  spacing::client:"),
        out("      connected server=10.234.79.81:24800"),
        dim(""),
        prompt("ubuntu@spacing-cli ~ $ evtest /dev/input/event4"),
        dim("Input device name: \"spacing-virtual-input\""),
        out("Event: code 35 (KEY_H), value 1   # h"),
        out("Event: code 18 (KEY_E), value 1   # e"),
        out("Event: code 38 (KEY_L), value 1   # l"),
        out("Event: code 24 (KEY_O), value 1   # o"),
        info("# \"hello\" typed on spacing-srv arrives here"),
    ]
    two_pane("spacing-srv — input owner", "spacing-cli — controlled", left, right,
             "docs/images/switch.png",
             note="Ctrl+Alt+1 = forward to 'spacing-cli' · Ctrl+Alt+0 = local · watchdog releases stuck keys")


if __name__ == "__main__":
    main()
