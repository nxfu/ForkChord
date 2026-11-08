"""Standalone visual mock of the fullscreen player's ambience at the r5
values — same structure as FullScreenPlayer.tsx + global.css, at the app's
own scale (the 460px cover it renders at 1440x900, blur 90/32px, melt-mask's
4.5% feather): the app's scrimmed full-cover blur for the backdrop, then
behind the sleeve a wash (blur 90 saturate 1.6 @ 0.50, wash-masked), the
artwork twice at the cover's own box (blur 32 saturate 1.25 @ 0.80, blur 32
contrast 1.4 @ 0.30) and the sleeve itself feathered by melt-mask. Renders
r4 (wash reached half a cover-length out) against r5 (twice the reach),
plus a mock of the animated sleeve — the ambience now mirrors the clip's
own frames, shown here as a hue-rotated clip over the same still — then
crops each cover's left edge at 2x, where a line of separation would show
and where the glow's reach reads.
"""

import pathlib

from PIL import Image, ImageDraw, ImageFont
from playwright.sync_api import sync_playwright

WORK = pathlib.Path("/tmp/ambience_mock")
WORK.mkdir(exist_ok=True)

# A "cover" with distinct regions (hot left, cool right, dark bottom) so the
# melt at each edge reads differently — like a real sleeve.
img = Image.new("RGB", (600, 600))
d = ImageDraw.Draw(img)
for y in range(600):
    for x in range(0, 600, 10):
        r = int(230 - (x / 600) * 90)
        g = int(40 + (y / 600) * 60)
        b = int(150 + (x / 600) * 80 - (y / 600) * 40)
        d.rectangle([x, y, x + 10, y], fill=(min(r, 255), min(g, 255), max(b, 0)))
d.ellipse([180, 160, 420, 400], fill=(250, 240, 220))
# A dark coda along the bottom, blended so no hard bar tests the melt with an
# edge the artwork itself drew.
px = img.load()
for y in range(460, 600):
    t = (y - 460) / 140
    for x in range(600):
        r0, g0, b0 = px[x, y]
        px[x, y] = (int(r0 * (1 - t) + 20 * t), int(g0 * (1 - t) + 18 * t), int(b0 * (1 - t) + 30 * t))
img.save(WORK / "cover.png")

# ArtworkBackdrop's steady state: the full-cover blur under SCRIM_FULL_BLUR.
SCRIM = "linear-gradient(180deg, rgb(0 0 0 / 0.34), rgb(0 0 0 / 0.48) 55%, rgb(0 0 0 / 0.64))"

# A 460px cover in a window-sized crop of the backdrop, as at 1440x900 —
# sized so the r5 wash's fade completes inside the card on every side
# (it ends a full cover-length out, then has to have somewhere to die).
ART, STAGE_W, STAGE_H, ART_TOP = 460, 1520, 1560, 500


def cross(stops: str) -> str:
    """A mask's two-axis cross, per stop pair."""
    grads = ", ".join(f"linear-gradient({ax}, {stops})" for ax in ("to right", "to bottom"))
    return f"mask-image: {grads}; mask-composite: intersect;"


def melt_mask(feather: str) -> str:
    return cross(f"transparent, #000 {feather}, #000 calc(100% - {feather}), transparent")


# Per variant: name, wash (filter, opacity, inset), and an optional tint that
# stands in for "the ambience mirrors the clip's frames, not the still" — a
# hue-rotated clip laid over the still, with every ambience copy rotated to
# match it, exactly as the canvas mirrors do in the app.
VARIANTS = [
    ("r4 — wash faded half a cover out", ("blur(90px) saturate(1.6)", 0.50, "-45%", "190%"), None),
    ("r5 — twice the reach", ("blur(90px) saturate(1.6)", 0.50, "-100%", "300%"), None),
    ("r5 — animated: the ambience follows the clip", ("blur(90px) saturate(1.6)", 0.50, "-100%", "300%"), "hue-rotate(140deg)"),
]

COLOUR = ("blur(32px) saturate(1.25)", 0.80)
PUNCH = ("blur(32px) contrast(1.4)", 0.30)
FEATHER = "20.7px"


def layer(extra: str, src: str = "cover.png") -> str:
    return f'<img class="layer" src="{src}" style="{extra}">'


cards = ""
for name, wash, tint in VARIANTS:
    f, o, inset, size = wash
    filt = f"{f} {tint}" if tint else f
    # An img is a replaced element: positioned by inset alone it keeps its
    # intrinsic size — exactly the bug the r5 wash sizing fixes in the app.
    # The explicit width/height is what makes the box reach at all.
    wash_html = layer(
        f"position: absolute; inset: {inset}; width: {size}; height: {size}; object-fit: cover; filter: {filt}; opacity: {o}; {cross('transparent, #000 20%, #000 80%, transparent')}"
    )
    colour = f"filter: {COLOUR[0]} {tint or ''}; opacity: {COLOUR[1]}".rstrip().replace(" ;", ";")
    punch = f"filter: {PUNCH[0]} {tint or ''}; opacity: {PUNCH[1]}".rstrip().replace(" ;", ";")
    # The top layer is the still, feathered — or, when tinted, the clip
    # playing over it (the canvas mirror's colours come from this frame).
    sleeve = f"background: #ffffff12; {melt_mask(FEATHER)}" + (f" filter: {tint};" if tint else "")
    cards += f"""
    <figure>
      <figcaption>{name}</figcaption>
      <div class="stage">
        <div class="backdrop"><div class="backdrop-blur"></div><div class="scrim"></div></div>
        <div class="art">
          {wash_html}
          {layer(colour)}
          {layer(punch)}
          {layer(sleeve)}
        </div>
      </div>
    </figure>"""

html = f"""<!doctype html><html><head><style>
  body {{ margin: 0; background: #101014; color: #eee; font: 14px sans-serif;
         display: flex; gap: 28px; padding: 28px; }}
  figure {{ margin: 0; text-align: center; }}
  figcaption {{ margin-bottom: 10px; opacity: .7; max-width: {STAGE_W}px }}
  .stage {{ position: relative; width: {STAGE_W}px; height: {STAGE_H}px; overflow: hidden;
           border-radius: 8px; }}
  .backdrop {{ position: absolute; inset: 0; background: #121212; overflow: hidden; }}
  .backdrop-blur {{ position: absolute; inset: -60px;
                   background: url(cover.png) center / cover; filter: blur(20px); }}
  .scrim {{ position: absolute; inset: 0; background: {SCRIM}; }}
  .art {{ position: absolute; left: 50%; top: {ART_TOP}px; width: {ART}px; height: {ART}px;
         margin-left: -{ART // 2}px; border-radius: 10px; }}
  .layer {{ position: absolute; inset: 0; object-fit: cover; }}
</style></head><body>{cards}</body></html>"""
(WORK / "index.html").write_text(html)

with sync_playwright() as p:
    browser = p.chromium.launch()
    page = browser.new_page(viewport={"width": len(VARIANTS) * STAGE_W + (len(VARIANTS) + 1) * 28, "height": STAGE_H + 160})
    page.goto(f"file://{WORK}/index.html")
    page.wait_for_timeout(600)
    shot = WORK / "row.png"
    page.screenshot(path=str(shot), full_page=True)
    # Exact on-screen rects of each cover box, for the edge crops.
    arts = page.evaluate(
        """[...document.querySelectorAll('.art')].map((el) => {
             const r = el.getBoundingClientRect();
             return { x: r.x, y: r.y, w: r.width, h: r.height };
           })"""
    )
    browser.close()

row = Image.open(shot).convert("RGB")

# The edge crops: 300px around each cover's left edge midpoint, blown up 2x —
# wide enough that the r4 glow visibly dies inside the crop while r5's
# carries on.
crop_side, scale, pad, label_h = 300, 2, 28, 36
zoomed = [
    row.crop((int(a["x"] - crop_side / 2), int(a["y"] + a["h"] / 2 - crop_side / 2),
              int(a["x"] - crop_side / 2 + crop_side), int(a["y"] + a["h"] / 2 + crop_side / 2)))
    for a in arts
]
strip_w = pad + len(zoomed) * (crop_side * scale) + (len(zoomed) - 1) * pad + pad

# The row goes out at half scale (it is twice the delivered width); the crops
# are taken at full res first, so the zoom stays honest.
row_small = row.resize((row.width // 2, row.height // 2), Image.LANCZOS)
final = Image.new(
    "RGB",
    (max(row_small.width, strip_w), row_small.height + label_h + crop_side * scale + pad),
    (16, 16, 20),
)
final.paste(row_small, (0, 0))
draw = ImageDraw.Draw(final)
try:
    font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 15)
except OSError:
    font = ImageFont.load_default()
draw.text((pad, row_small.height + 9), "each cover's left edge, x2 — where a line of separation would live, and how far the glow carries", fill=(160, 160, 170), font=font)
for i, z in enumerate(zoomed):
    big = z.resize((crop_side * scale, crop_side * scale), Image.BILINEAR)
    final.paste(big, (pad + i * (crop_side * scale + pad), row_small.height + label_h))
final.save(WORK / "ambience-reach-compare.png")
print("rendered:", WORK / "ambience-reach-compare.png", final.size)
