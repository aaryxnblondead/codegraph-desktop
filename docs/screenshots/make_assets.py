from PIL import Image

SRC = "docs/screenshots/live-sync.png"
img = Image.open(SRC).convert("RGB")
W, H = img.size  # 1450 x 940

# ---- social preview: 1280 x 640, centred on the graph -------------------
sw, sh = 1280, 640
left = (W - sw) // 2
top = 120  # keep header + graph, drop some list padding
social = img.crop((left, top, left + sw, top + sh))
social.save("docs/screenshots/social-preview.png", optimize=True)
print("social-preview.png", social.size)

# ---- demo gif: slow zoom toward the highlighted node --------------------
# The touched node sits around (880, 315) in the 1450x940 screenshot.
target = (880, 315)
frames = 16
width_out = 900
durations = []

raw = []
for i in range(frames):
    t = i / (frames - 1)
    ease = t * t * (3 - 2 * t)            # smoothstep
    scale = 1.0 + 0.75 * ease             # 1.0 -> 1.75 zoom
    cw, ch = int(W / scale), int(H / scale)
    # centre drifts from the image centre to the node
    cx = W / 2 + (target[0] - W / 2) * ease
    cy = H / 2 + (target[1] - H / 2) * ease
    x0 = max(0, min(W - cw, int(cx - cw / 2)))
    y0 = max(0, min(H - ch, int(cy - ch / 2)))
    frame = img.crop((x0, y0, x0 + cw, y0 + ch))
    frame = frame.resize(
        (width_out, int(ch * width_out / cw)), Image.LANCZOS
    )
    raw.append(frame)
    durations.append(130)

# hold on the final frame briefly
raw.append(raw[-1])
durations.append(900)

# palette-quantise for a small, clean GIF
paletted = [f.quantize(colors=128, method=Image.MEDIANCUT) for f in raw]
paletted[0].save(
    "docs/screenshots/demo.gif",
    save_all=True,
    append_images=paletted[1:],
    duration=durations,
    loop=0,
    optimize=True,
    disposal=2,
)
import os
print("demo.gif", paletted[0].size, os.path.getsize("docs/screenshots/demo.gif") // 1024, "KB")
