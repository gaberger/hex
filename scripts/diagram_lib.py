"""Hand-rolled diagram SVGs. Vertical flow, sized for a phone."""
from xml.sax.saxutils import escape

W          = 560      # viewBox width. Text is sized relative to this.
PAD        = 14
FS         = 25       # body text
FS_SM      = 21       # edge labels
LH         = 32       # line height
BOX_PAD_Y  = 18
GAP        = 34       # vertical gap between boxes
ARROW      = 20

PALETTE = {
    # role:      (light fill, light stroke, dark fill, dark stroke)
    "step":     ("#ffffff", "#8c959f", "#161b22", "#6e7681"),
    "check":    ("#ddf4ff", "#54aeff", "#121d2f", "#4493f8"),
    "good":     ("#dafbe1", "#4ac26b", "#0f2417", "#3fb950"),
    "bad":      ("#ffebe9", "#ff8182", "#25171c", "#f85149"),
    "warn":     ("#fff8c5", "#d4a72c", "#272115", "#d29922"),
    "accent":   ("#fbefff", "#c297ff", "#1d1b28", "#a371f7"),
}
TEXT   = ("#1f2328", "#e6edf3")
LINE   = ("#6e7781", "#8b949e")
MUTED  = ("#57606a", "#9198a1")

def box_w(lines, fs=FS):
    return max(int(len(l) * fs * 0.56) + 2 * 22 for l in lines)

def box_h(lines):
    return len(lines) * LH + 2 * BOX_PAD_Y - (LH - FS)

class Doc:
    def __init__(self, dark):
        self.d = dark
        self.parts = []
        self.maxy = 0
    def c(self, quad):
        return (quad[2], quad[3]) if self.d else (quad[0], quad[1])
    def box(self, cx, y, lines, role="step"):
        fill, stroke = self.c(PALETTE[role])
        w, h = box_w(lines), box_h(lines)
        x = cx - w / 2
        self.parts.append(
            f'<rect x="{x:.0f}" y="{y:.0f}" width="{w}" height="{h}" rx="10" '
            f'fill="{fill}" stroke="{stroke}" stroke-width="2"/>'
        )
        ty = y + BOX_PAD_Y + FS * 0.78
        for i, l in enumerate(lines):
            self.parts.append(
                f'<text x="{cx:.0f}" y="{ty + i*LH:.0f}" text-anchor="middle" '
                f'font-size="{FS}" fill="{TEXT[self.d]}" '
                f'font-family="-apple-system,BlinkMacSystemFont,Segoe UI,Helvetica,Arial,sans-serif">'
                f'{escape(l)}</text>'
            )
        self.maxy = max(self.maxy, y + h)
        return (cx, y, w, h)
    def arrow(self, x1, y1, x2, y2, label=None, dashed=False):
        dash = ' stroke-dasharray="6 5"' if dashed else ''
        self.parts.append(
            f'<path d="M {x1:.0f} {y1:.0f} L {x2:.0f} {y2:.0f}" stroke="{LINE[self.d]}" '
            f'stroke-width="2.5" fill="none" marker-end="url(#a)"{dash}/>'
        )
        if label:
            # Beside the line, never on top of it. A chip centred on the arrow
            # hides the arrowhead, which is the part that carries the meaning.
            mx, my = (x1 + x2) / 2 + 12, (y1 + y2) / 2
            self.parts.append(
                f'<text x="{mx:.0f}" y="{my + FS_SM*0.36:.0f}" text-anchor="start" '
                f'font-size="{FS_SM}" fill="{MUTED[self.d]}" '
                f'font-family="-apple-system,BlinkMacSystemFont,Segoe UI,Helvetica,Arial,sans-serif">'
                f'{escape(label)}</text>'
            )
        self.maxy = max(self.maxy, y2)
    def render(self, width=W):
        h = self.maxy + PAD
        return (
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {h:.0f}" '
            f'width="{width}" height="{h:.0f}" role="img">'
            f'<defs><marker id="a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" '
            f'markerHeight="7" orient="auto-start-reverse">'
            f'<path d="M 0 0 L 10 5 L 0 10 z" fill="{LINE[self.d]}"/></marker></defs>'
            + "".join(self.parts) + "</svg>"
        )
