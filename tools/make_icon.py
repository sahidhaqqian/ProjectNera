"""Convert the supplied official artwork; preserve its composition and alpha."""
import argparse
from pathlib import Path
from PIL import Image

parser = argparse.ArgumentParser()
parser.add_argument("source", type=Path)
parser.add_argument("--output", type=Path, default=Path("assets/branding"))
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
with Image.open(args.source) as source:
    image = source.convert("RGBA")
    if image.width != image.height:
        canvas = Image.new("RGBA", (max(image.size),) * 2)
        canvas.alpha_composite(image, ((canvas.width-image.width)//2, (canvas.height-image.height)//2))
        image = canvas
    image.save(args.output / "nera-logo.png")
    image.save(args.output / "nera.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
    with Image.open(args.output / "nera.ico") as icon:
        assert icon.ico.sizes() == {(s, s) for s in (16, 24, 32, 48, 64, 128, 256)}
    print("Official icon: 16, 24, 32, 48, 64, 128, 256; RGBA artwork retained")
