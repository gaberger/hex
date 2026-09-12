"""Build the documentation diagrams.

Usage: python3 scripts/build-diagrams.py

Hand-rolled SVG rather than a diagram language. Mermaid is rendered by GitHub
in a browser, so it shows as source code in the mobile app, and its layout is
not controllable enough to stay readable at phone width. These are laid out
here, in two themes, and committed.
"""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from diagram_lib import Doc, box_h, GAP, ARROW, W

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".github", "assets", "diagrams")
os.makedirs(OUT, exist_ok=True)

def drift(d):
    cx = W/2; y = 14
    b1 = d.box(cx, y, ["agent writes code"], "step");            y1 = y + b1[3]
    y = y1 + GAP
    b2 = d.box(cx, y, ["tests pass?"], "check");                 y2 = y + b2[3]
    d.arrow(cx, y1, cx, y - 6)
    # the retry loop, drawn to the left
    lx = cx - b2[2]/2 - 46
    d.parts.append(f'<path d="M {cx - b2[2]/2:.0f} {y + b2[3]/2:.0f} H {lx} V {y1 - b1[3]/2:.0f} H {cx - b1[2]/2:.0f}" '
                   f'stroke="{d.c(("#6e7781","","#8b949e",""))[0]}" stroke-width="2.5" fill="none" marker-end="url(#a)"/>')
    d.parts.append(f'<text x="{lx - 8:.0f}" y="{(y + y1)/2:.0f}" text-anchor="end" font-size="21" '
                   f'fill="{"#9198a1" if d.d else "#57606a"}" font-family="-apple-system,BlinkMacSystemFont,Segoe UI,Helvetica,Arial,sans-serif">no</text>')
    y = y2 + GAP
    b3 = d.box(cx, y, ["ship it"], "step");                      y3 = y + b3[3]
    d.arrow(cx, y2, cx, y - 6, "yes")
    y = y3 + GAP
    b4 = d.box(cx, y, ["shape drifts,", "nothing checks it"], "warn"); y4 = y + b4[3]
    d.arrow(cx, y3, cx, y - 6)
    y = y4 + GAP
    d.box(cx, y, ["week 12:", "nothing is where", "it belongs"], "bad")
    d.arrow(cx, y4, cx, y - 6)

def pipeline(d):
    cx = W/2; y = 14
    steps = [
        (["1. FLOOR", "deterministic skeleton"], "step", None),
        (["floor gate", "does it run here?"], "check", None),
        (["2. BUILD", "designs, red-teamed,", "built to the gate"], "step", "yes"),
        (["gate", "does it run?"], "check", None),
        (["architecture grade", "is it the right shape?"], "accent", "yes"),
        (["3. SHIP", "with rules that travel"], "good", "grade A+"),
    ]
    prev = None
    for lines, role, lbl in steps:
        if prev is not None:
            d.arrow(cx, prev, cx, y - 6, lbl)
        b = d.box(cx, y, lines, role)
        prev = y + b[3]
        y = prev + GAP

def hexagon(d):
    cx = W/2; y = 14
    rows = [
        (["adapters/primary", "HTTP, CLI, UI"], "accent"),
        (["usecases", "orchestration"], "step"),
        (["ports", "interfaces"], "check"),
        (["domain", "pure logic,", "imports nothing"], "good"),
    ]
    ys = []
    for lines, role in rows:
        b = d.box(cx, y, lines, role); ys.append((y, y + b[3], b[2])); y = y + b[3] + GAP
    d.arrow(cx, ys[0][1], cx, ys[1][0] - 6, "ports only")
    d.arrow(cx, ys[1][1], cx, ys[2][0] - 6)
    d.arrow(cx, ys[2][1], cx, ys[3][0] - 6)
    # secondary adapters, routed up the left margin into ports
    b = d.box(cx, y, ["adapters/secondary", "database, files, APIs"], "accent")
    sec_top, sec_mid = y, y + b[3]/2
    col = "#8b949e" if d.d else "#6e7781"
    mut = "#9198a1" if d.d else "#57606a"
    d.parts.append(
        f'<path d="M {cx - b[2]/2:.0f} {sec_mid:.0f} H 26 V {ys[2][0] + 26:.0f} H {cx - ys[2][2]/2 - 4:.0f}" '
        f'stroke="{col}" stroke-width="2.5" fill="none" marker-end="url(#a)"/>')
    d.parts.append(
        f'<text x="34" y="{(sec_mid + ys[2][0])/2:.0f}" font-size="21" fill="{mut}" '
        f'font-family="-apple-system,BlinkMacSystemFont,Segoe UI,Helvetica,Arial,sans-serif">ports only</text>')
    y = y + b[3] + GAP
    # the composition root is the only thing allowed to touch an adapter
    b2 = d.box(cx, y, ["composition root", "the only file that may", "import an adapter"], "warn")
    d.parts.append(
        f'<path d="M {cx + b2[2]/2 + 4:.0f} {y + b2[3]/2:.0f} H {W - 22} V {ys[0][0] + 26:.0f} H {cx + ys[0][2]/2 + 4:.0f}" '
        f'stroke="{col}" stroke-width="2.5" fill="none" stroke-dasharray="6 5" marker-end="url(#a)"/>')

def loop(d):
    cx = W/2; y = 14
    items = [(["task, graph context,", "ranked lessons"], "step"),
             (["reason", "read and verify tools"], "step"),
             (["propose_edit"], "accent"),
             (["run the evidence", "command"], "check"),
             (["commit"], "good")]
    ys = []
    for lines, role in items:
        b = d.box(cx, y, lines, role); ys.append((y, y + b[3], b[2])); y = y + b[3] + GAP
    for i in range(len(ys) - 1):
        lbl = "exit 0" if i == 3 else None
        d.arrow(cx, ys[i][1], cx, ys[i+1][0] - 6, lbl)
    # revert loop back to reason
    rx = cx + ys[3][2]/2 + 46
    d.parts.append(f'<path d="M {cx + ys[3][2]/2:.0f} {ys[3][0] + 26:.0f} H {rx:.0f} V {ys[1][0] + 26:.0f} H {cx + ys[1][2]/2:.0f}" '
                   f'stroke="{"#8b949e" if d.d else "#6e7781"}" stroke-width="2.5" fill="none" marker-end="url(#a)" stroke-dasharray="6 5"/>')
    d.parts.append(f'<text x="{rx + 8:.0f}" y="{(ys[1][0] + ys[3][0])/2 + 30:.0f}" font-size="21" '
                   f'fill="{"#9198a1" if d.d else "#57606a"}" font-family="-apple-system,BlinkMacSystemFont,Segoe UI,Helvetica,Arial,sans-serif">revert</text>')

def crates(d):
    cx = W/2; y = 14
    rows = [(["hex-cli", "the binary, the only", "composition root"], "warn"),
            (["hex-exec", "agent loop, harness,", "guarded tools"], "step"),
            (["hex-infer", "every adapter,", "tier resolution"], "accent"),
            (["hex-core", "contract surface", "zero runtime deps"], "good")]
    ys = []
    for lines, role in rows:
        b = d.box(cx, y, lines, role); ys.append((y, y + b[3])); y = y + b[3] + GAP
    for i in range(len(ys) - 1):
        d.arrow(cx, ys[i][1], cx, ys[i+1][0] - 6)

for name, fn in [("drift", drift), ("pipeline", pipeline), ("hexagon", hexagon),
                 ("loop", loop), ("crates", crates)]:
    for dark in (False, True):
        d = Doc(dark); fn(d)
        suffix = "dark" if dark else "light"
        open(f"{OUT}/{name}-{suffix}.svg", "w").write(d.render())
    print(f"  {name}: light + dark")
