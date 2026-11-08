#!/usr/bin/env python3
"""Generate a five-page "Creative Journal" page-spec for ignite-import (stdlib only).

    python3 gen_creative_journal.py > creative-journal.json

Coordinates are in page-width units (x 0..1, y 0..1.333), see src/spec.rs. Everything is drawn
as ordinary ink strokes except the paper (lined / dot / grid), which is a background.
"""
import json
import math
import sys

LINE0_PX, STEP_PX, PX_W = 200, 83, 1860  # lined paper geometry (src/paper.rs)


def tw(text, cap):
    """Width of `text` in the built-in stroke font (src/font.rs: advance 11, space 6, cap 12)."""
    return sum(6 if c == " " else 11 for c in text) / 12 * cap


def label(text, x, y, size):
    return {"text": text, "x": round(x, 4), "y": round(y, 4), "size": size}


def centered(text, cx, cy, size):
    return label(text, cx - tw(text, size) / 2, cy - size / 2, size)


def rect(x, y, w, h):
    return {"kind": "rect", "x": round(x, 4), "y": round(y, 4), "w": round(w, 4), "h": round(h, 4)}


def ellipse(cx, cy, rx, ry):
    return {"kind": "ellipse", "cx": round(cx, 4), "cy": round(cy, 4), "rx": rx, "ry": ry}


def line(a, b):
    return {"kind": "line", "from": [round(a[0], 4), round(a[1], 4)], "to": [round(b[0], 4), round(b[1], 4)]}


def arrow(a, b):
    return {"kind": "arrow", "from": [round(a[0], 4), round(a[1], 4)], "to": [round(b[0], 4), round(b[1], 4)]}


def lined_y(k):
    return (LINE0_PX + STEP_PX * k) / PX_W


def rect_edge(center, half, toward):
    """Point where the ray from `center` toward `toward` leaves the rectangle (half = (hw, hh))."""
    dx, dy = toward[0] - center[0], toward[1] - center[1]
    t = min(half[0] / abs(dx) if dx else 1e9, half[1] / abs(dy) if dy else 1e9)
    return (center[0] + dx * t, center[1] + dy * t)


def ellipse_edge(c, rx, ry, toward):
    th = math.atan2((toward[1] - c[1]) / ry, (toward[0] - c[0]) / rx)
    return (c[0] + rx * math.cos(th), c[1] + ry * math.sin(th))


def mind_map():
    C, RX, RY = (0.5, 0.67), 0.17, 0.07
    nodes = [  # (label, node centre, expansion-circle centre)
        ("COMPUTE", (0.22, 0.40), (0.10, 0.27)),
        ("DATA", (0.78, 0.40), (0.90, 0.27)),
        ("AGENTS", (0.15, 0.67), (0.15, 0.52)),
        ("ROBOTICS", (0.85, 0.67), (0.85, 0.52)),
        ("SAFETY", (0.22, 0.94), (0.10, 1.08)),
        ("WORK", (0.78, 0.94), (0.90, 1.08)),
    ]
    NW, NH, R = 0.24, 0.075, 0.032
    shapes = [ellipse(*C, RX, RY)]
    labels = [
        label("MIND MAP: FUTURE OF AI", 0.06, 0.06, 0.035),
        label("ADD BRANCHES BY HAND", 0.06, 0.13, 0.02),
        centered("FUTURE OF AI", *C, 0.024),
        label("WHAT DID I MISS?", 0.06, 1.22, 0.022),
    ]
    for name, p, q in nodes:
        shapes.append(rect(p[0] - NW / 2, p[1] - NH / 2, NW, NH))
        labels.append(centered(name, p[0], p[1], 0.018))
        shapes.append(line(rect_edge(p, (NW / 2, NH / 2), C), ellipse_edge(C, RX, RY, p)))
        shapes.append(ellipse(q[0], q[1], R, R))  # empty node to fill in by hand
        d = (q[0] - p[0], q[1] - p[1])
        n = math.hypot(*d)
        shapes.append(line(rect_edge(p, (NW / 2, NH / 2), q), (q[0] - d[0] / n * R, q[1] - d[1] / n * R)))
    return {"paper": "blank", "shapes": shapes, "labels": labels}


def daily_journal():
    L = lined_y
    labels = [
        label("TODAY", 0.06, 0.085, 0.04),
        label("DATE:", 0.06, L(3) - 0.026, 0.02),
        label("MOOD:", 0.55, L(3) - 0.026, 0.02),
        label("THREE THINGS I NOTICED", 0.06, L(5) - 0.03, 0.022),
        label("1.", 0.06, L(7) - 0.026, 0.02),
        label("2.", 0.06, L(9) - 0.026, 0.02),
        label("3.", 0.06, L(11) - 0.026, 0.02),
        label("ONE THING TO REMEMBER", 0.06, L(13) - 0.03, 0.022),
        label("A QUESTION FOR TOMORROW", 0.06, L(21) - 0.03, 0.022),
    ]
    shapes = [rect(0.72, 0.08, 0.22, 0.05), ellipse(0.83, 0.105, 0.012, 0.012)]
    return {"paper": "lined", "shapes": shapes, "labels": labels}


def sketch_prompt():
    labels = [
        label("SKETCH PROMPT", 0.06, 0.04, 0.04),
        label("DRAW SOMETHING YOU SAW TODAY", 0.06, 0.105, 0.02),
        label("WHAT IS IT? WHY DID IT STICK?", 0.06, 1.03, 0.02),
    ]
    shapes = [rect(0.06, 0.17, 0.88, 0.80), rect(0.06, 1.08, 0.88, 0.19)]
    return {"paper": "dot", "shapes": shapes, "labels": labels}


def reading_notes():
    labels = [
        label("READING NOTES", 0.06, 0.04, 0.04),
        label("BOOK:", 0.06, 0.125, 0.02),
        label("AUTHOR:", 0.06, 0.19, 0.02),
        label("KEY IDEAS", 0.06, 0.27, 0.025),
        label("FAVOURITE QUOTE", 0.06, 0.80, 0.025),
        label("RATING", 0.06, 1.12, 0.02),
    ]
    shapes = [line((0.20, 0.15), (0.94, 0.15)), line((0.24, 0.215), (0.94, 0.215))]
    for i in range(3):
        y = 0.32 + i * 0.15
        shapes += [rect(0.06, y, 0.88, 0.12), ellipse(0.11, y + 0.06, 0.03, 0.03)]
        labels.append(centered(str(i + 1), 0.11, y + 0.06, 0.02))
    shapes.append(rect(0.06, 0.85, 0.88, 0.22))
    shapes += [ellipse(0.26 + i * 0.09, 1.13, 0.03, 0.03) for i in range(5)]
    return {"paper": "grid", "shapes": shapes, "labels": labels}


def music_page():
    labels = [
        label("MUSIC DISCOVERY", 0.06, 0.04, 0.04),
        label("ARTIST:", 0.06, 0.125, 0.02),
        label("ALBUM:", 0.06, 0.19, 0.02),
        label("WHY I LISTENED:", 0.06, 0.27, 0.02),
        label("A MELODY I WANT TO KEEP", 0.06, 0.52, 0.022),
        label("HOW IT MADE ME FEEL", 0.06, 1.08, 0.022),
    ]
    shapes = [line((0.23, 0.15), (0.94, 0.15)), line((0.22, 0.215), (0.94, 0.215))]
    for top in (0.58, 0.72, 0.86):  # three five-line staves
        for k in range(5):
            y = top + k * 0.018
            shapes.append(line((0.06, y), (0.94, y)))
        shapes += [line((0.06, top), (0.06, top + 0.072)), line((0.94, top), (0.94, top + 0.072))]
    shapes.append(rect(0.06, 1.13, 0.88, 0.14))
    return {"paper": "blank", "shapes": shapes, "labels": labels}


# Ignite reserves panel rows 0..140 and 2340..2480 for its chrome; ignite-import rejects ink
# there (safe y range 0.0774..1.2559 in page-width units). The unlined pages above were laid out on
# the full page, so they are scaled uniformly (circles stay circles) into the safe area.
SAFE_TOP, SAFE_BOTTOM, PAGE_H = 0.0774, 1.2559, 4 / 3
FIT = (SAFE_BOTTOM - SAFE_TOP - 0.004) / PAGE_H
FIT_X0, FIT_Y0 = (1 - FIT) / 2, SAFE_TOP + 0.002


def fit(page):
    X = lambda x: round(FIT_X0 + x * FIT, 4)
    Y = lambda y: round(FIT_Y0 + y * FIT, 4)
    S = lambda v: round(v * FIT, 4)
    for sh in page.get("shapes", []):
        if sh["kind"] == "rect":
            sh.update(x=X(sh["x"]), y=Y(sh["y"]), w=S(sh["w"]), h=S(sh["h"]))
        elif sh["kind"] == "ellipse":
            sh.update(cx=X(sh["cx"]), cy=Y(sh["cy"]), rx=S(sh["rx"]), ry=S(sh["ry"]))
        else:
            sh.update({k: [X(sh[k][0]), Y(sh[k][1])] for k in ("from", "to")})
    for lb in page.get("labels", []):
        lb.update(x=X(lb["x"]), y=Y(lb["y"]), size=S(lb["size"]))
    return page


def main():
    spec = {
        "notebook": {"title": "Creative Journal"},
        # The lined page is aligned to its paper lines, so it is placed by hand instead of scaled.
        "pages": [fit(mind_map()), daily_journal(), fit(sketch_prompt()), fit(reading_notes()), fit(music_page())],
    }
    json.dump(spec, sys.stdout, indent=1)
    print()


if __name__ == "__main__":
    main()
