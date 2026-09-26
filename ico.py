# ico.py
"""
生成 DeployTool 多分辨率应用图标 (app.ico 和 app.png)。
使用 Pillow 读取 high-res app.png 导出 16/24/32/48/64/128/256 多尺寸 ICO。

依赖：pip install pillow
用法：python ico.py
"""
from pathlib import Path
from PIL import Image

ASSETS = Path(__file__).parent / "assets"
PNG_PATH = ASSETS / "app.png"
ICO_PATH = ASSETS / "app.ico"


def build_ico():
    if not PNG_PATH.exists():
        print(f"错误: 未找到 {PNG_PATH}")
        return

    img = Image.open(PNG_PATH).convert("RGBA")
    sizes = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    img.save(ICO_PATH, format="ICO", sizes=sizes)
    print(f"图标已成功导出: {ICO_PATH.resolve()}")
    print(f"包含尺寸: {sizes}")


if __name__ == "__main__":
    build_ico()