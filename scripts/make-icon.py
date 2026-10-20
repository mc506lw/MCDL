"""生成 MCDL 应用图标源图（1024×1024）。

风格来自 endfield-ui 变体卡：墨底 #161616、ice 强调色 #7DE6FF、切角。
图形是「向下箭头」，下载/安装的通用语义，32px 下仍然认得出。
"""

from PIL import Image, ImageDraw

SIZE = 1024
INK = (22, 22, 22, 255)
ICE = (125, 230, 255, 255)
CUT = 132  # 右下角切掉的边长


def main() -> None:
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # 底板：右下角切一个角，其余是墨色方块
    draw.polygon(
        [(0, 0), (SIZE, 0), (SIZE, SIZE - CUT), (SIZE - CUT, SIZE), (0, SIZE)],
        fill=INK,
    )

    # 箭头：竖杆 + 三角头。几何形，不用圆角。
    shaft_w = 196
    shaft_top, shaft_bottom = 196, 560
    cx = SIZE // 2
    draw.rectangle(
        [cx - shaft_w // 2, shaft_top, cx + shaft_w // 2, shaft_bottom],
        fill=ICE,
    )
    head_half, head_top, head_bottom = 268, 512, 812
    draw.polygon(
        [(cx - head_half, head_top), (cx + head_half, head_top), (cx, head_bottom)],
        fill=ICE,
    )

    # 落点：一条细横线，表示「装到这里」
    draw.rectangle([cx - head_half, 884, cx + head_half, 920], fill=ICE)

    img.save("assets/icon-source.png")
    print("wrote assets/icon-source.png", img.size)


if __name__ == "__main__":
    main()
