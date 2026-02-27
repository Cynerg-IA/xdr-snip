"""Generate social preview image for XDR Snip GitHub repo.

Output: assets/social-preview.png (1280x640)
Run:    python3 assets/generate-preview.py
"""

from PIL import Image, ImageDraw, ImageFont
import os

# --- Config ---
WIDTH, HEIGHT = 1280, 640
BG_COLOR = (13, 17, 23)           # GitHub dark: #0d1117
ACCENT_COLOR = (0, 200, 220)      # Cyan (matches overlay selection border)
TEXT_COLOR = (230, 237, 243)       # Light gray: #e6edf3
MUTED_COLOR = (139, 148, 158)     # Muted: #8b949e
PILL_BG = (33, 38, 45)            # Pill background: #21262d
PILL_BORDER = (48, 54, 61)        # Pill border: #30363d

FEATURES = [
    "Frozen Capture",
    "JPEG Output",
    "Single EXE",
    "System Tray",
    "DPI-Aware",
]

FOOTER = "Windows 11  ·  Rust  ·  MIT License"

# --- Helpers ---
def get_font(size):
    """Try common system fonts, fall back to default."""
    candidates = [
        "C:/Windows/Fonts/segoeui.ttf",
        "C:/Windows/Fonts/segoeuib.ttf",
        "C:/Windows/Fonts/arial.ttf",
        "C:/Windows/Fonts/calibri.ttf",
    ]
    for path in candidates:
        if os.path.exists(path):
            return ImageFont.truetype(path, size)
    return ImageFont.load_default()


def get_bold_font(size):
    """Try bold variants first."""
    bold_candidates = [
        "C:/Windows/Fonts/segoeuib.ttf",
        "C:/Windows/Fonts/arialbd.ttf",
        "C:/Windows/Fonts/calibrib.ttf",
    ]
    for path in bold_candidates:
        if os.path.exists(path):
            return ImageFont.truetype(path, size)
    return get_font(size)


def draw_rounded_rect(draw, xy, radius, fill, outline=None):
    """Draw a rounded rectangle."""
    x0, y0, x1, y1 = xy
    draw.rounded_rectangle(xy, radius=radius, fill=fill, outline=outline)


def text_width(draw, text, font):
    """Get text bounding box width."""
    bbox = draw.textbbox((0, 0), text, font=font)
    return bbox[2] - bbox[0]


# --- Main ---
def generate():
    img = Image.new("RGB", (WIDTH, HEIGHT), BG_COLOR)
    draw = ImageDraw.Draw(img)

    # Fonts
    title_font = get_bold_font(72)
    tagline_font = get_font(28)
    pill_font = get_font(22)
    footer_font = get_font(20)

    # --- Accent line at top ---
    draw.rectangle([(0, 0), (WIDTH, 4)], fill=ACCENT_COLOR)

    # --- Title: "XDR Snip" ---
    title = "XDR Snip"
    title_bbox = draw.textbbox((0, 0), title, font=title_font)
    title_w = title_bbox[2] - title_bbox[0]
    title_x = (WIDTH - title_w) // 2
    title_y = 120
    draw.text((title_x, title_y), title, fill=TEXT_COLOR, font=title_font)

    # --- Cyan underline below title ---
    line_y = title_y + (title_bbox[3] - title_bbox[1]) + 16
    line_half = 60
    draw.rectangle(
        [(WIDTH // 2 - line_half, line_y), (WIDTH // 2 + line_half, line_y + 3)],
        fill=ACCENT_COLOR,
    )

    # --- Tagline ---
    tagline = "Lightweight screenshot tool for Windows 11"
    tag_bbox = draw.textbbox((0, 0), tagline, font=tagline_font)
    tag_w = tag_bbox[2] - tag_bbox[0]
    tag_x = (WIDTH - tag_w) // 2
    tag_y = line_y + 30
    draw.text((tag_x, tag_y), tagline, fill=MUTED_COLOR, font=tagline_font)

    # --- Subtitle ---
    subtitle = "Select a region on a frozen screen, get a small JPEG"
    sub_bbox = draw.textbbox((0, 0), subtitle, font=tagline_font)
    sub_w = sub_bbox[2] - sub_bbox[0]
    sub_x = (WIDTH - sub_w) // 2
    sub_y = tag_y + 40
    draw.text((sub_x, sub_y), subtitle, fill=MUTED_COLOR, font=tagline_font)

    # --- Feature pills ---
    pill_h = 38
    pill_pad_x = 20
    pill_gap = 16
    pill_y = sub_y + 70

    # Calculate total width of all pills
    pill_widths = []
    for feat in FEATURES:
        w = text_width(draw, feat, pill_font) + pill_pad_x * 2
        pill_widths.append(w)
    total_pills_w = sum(pill_widths) + pill_gap * (len(FEATURES) - 1)
    pill_x = (WIDTH - total_pills_w) // 2

    for i, feat in enumerate(FEATURES):
        pw = pill_widths[i]
        x0 = pill_x
        y0 = pill_y
        x1 = x0 + pw
        y1 = y0 + pill_h

        draw_rounded_rect(draw, (x0, y0, x1, y1), radius=8, fill=PILL_BG, outline=PILL_BORDER)

        # Center text in pill
        tw = text_width(draw, feat, pill_font)
        tx = x0 + (pw - tw) // 2
        ty = y0 + (pill_h - (pill_font.size)) // 2 - 1
        draw.text((tx, ty), feat, fill=ACCENT_COLOR, font=pill_font)

        pill_x = x1 + pill_gap

    # --- Footer ---
    f_bbox = draw.textbbox((0, 0), FOOTER, font=footer_font)
    f_w = f_bbox[2] - f_bbox[0]
    f_x = (WIDTH - f_w) // 2
    f_y = HEIGHT - 60
    draw.text((f_x, f_y), FOOTER, fill=MUTED_COLOR, font=footer_font)

    # --- Save ---
    script_dir = os.path.dirname(os.path.abspath(__file__))
    output = os.path.join(script_dir, "social-preview.png")
    img.save(output, "PNG")
    print(f"Generated: {output} ({WIDTH}x{HEIGHT})")


if __name__ == "__main__":
    generate()
