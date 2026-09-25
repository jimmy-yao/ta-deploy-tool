# make_icon.py
"""
生成 DeployTool 图标：蓝色圆角方块 + 白色向上箭头 + 服务器堆叠线。
输出：assets/app.ico（含 16/24/32/48/64/128/256 多种尺寸）

依赖：pip install pillow
用法：python make_icon.py
"""
from pathlib import Path
from PIL import Image, ImageDraw

OUTPUT = Path(__file__).parent / "assets" / "app.ico"
SIZE = 256


def rounded_rect(draw, box, radius, fill):
    """Pillow 的 rounded_rectangle 在高版本才有，这里手写兼容。"""
    x0, y0, x1, y1 = box
    draw.rectangle([x0 + radius, y0, x1 - radius, y1], fill=fill)
    draw.rectangle([x0, y0 + radius, x1, y1 - radius], fill=fill)
    draw.pieslice([x0, y0, x0 + 2 * radius, y0 + 2 * radius], 180, 270, fill=fill)
    draw.pieslice([x1 - 2 * radius, y0, x1, y0 + 2 * radius], 270, 360, fill=fill)
    draw.pieslice([x0, y1 - 2 * radius, x0 + 2 * radius, y1], 90, 180, fill=fill)
    draw.pieslice([x1 - 2 * radius, y1 - 2 * radius, x1, y1], 0, 90, fill=fill)


def make_image() -> Image.Image:
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # ---------- 背景：圆角方块 + 垂直渐变 ----------
    # 先画纯色底，再叠一层渐变
    margin = 8
    radius = 44
    bg_box = [margin, margin, SIZE - margin, SIZE - margin]

    # 渐变：从 #2F80ED 到 #56CCF2
    grad = Image.new("RGBA", (1, SIZE), (0, 0, 0, 0))
    for y in range(SIZE):
        t = y / (SIZE - 1)
        r = int(0x2F + (0x56 - 0x2F) * t)
        g = int(0x80 + (0xCC - 0x80) * t)
        b = int(0xED + (0xF2 - 0xED) * t)
        grad.putpixel((0, y), (r, g, b, 255))
    grad = grad.resize((SIZE, SIZE))

    # 圆角遮罩
    mask = Image.new("L", (SIZE, SIZE), 0)
    mask_draw = ImageDraw.Draw(mask)
    rounded_rect(mask_draw, bg_box, radius, 255)

    img.paste(grad, (0, 0), mask)

    # ---------- 白色向上箭头 ----------
    cx = SIZE // 2
    arrow_color = (255, 255, 255, 255)

    # 箭头上半部分（三角形）
    tri_top = (cx, 60)
    tri_left = (cx - 58, 128)
    tri_right = (cx + 58, 128)
    draw.polygon([tri_top, tri_left, tri_right], fill=arrow_color)

    # 箭杆（矩形）
    shaft_w = 40
    shaft_top = 118
    shaft_bottom = 178
    draw.rectangle(
        [cx - shaft_w // 2, shaft_top, cx + shaft_w // 2, shaft_bottom],
        fill=arrow_color,
    )

    # ---------- 底部三条横线（服务器堆叠） ----------
    line_color = (255, 255, 255, 220)
    line_w = 96
    line_h = 10
    line_radius = 5
    line_positions = [196, 210, 224]

    for y in line_positions:
        rounded_rect(
            draw,
            [cx - line_w // 2, y, cx + line_w // 2, y + line_h],
            line_radius,
            line_color,
        )

    return img


def main():
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)

    img = make_image()
    sizes = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    img.save(OUTPUT, format="ICO", sizes=sizes)
    print(f"图标已生成: {OUTPUT.resolve()}")
    print(f"包含尺寸: {sizes}")

    # 顺便导出 png，方便预览或在 README 里用
    png_path = OUTPUT.with_suffix(".png")
    img.save(png_path, format="PNG")
    print(f"预览图已生成: {png_path.resolve()}")


if __name__ == "__main__":
    main()