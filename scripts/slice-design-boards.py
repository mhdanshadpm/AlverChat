from pathlib import Path
from PIL import Image, ImageStat
import json

root = Path(__file__).resolve().parents[1]
boards = sorted((root / 'design').glob('*.png'))
out = root / 'design' / 'frames'
out.mkdir(exist_ok=True)
colors = {}
for path in boards:
    image = Image.open(path).convert('RGB')
    width, height = image.size
    # Preserve each board as an 8-cell review sheet: crops make the illustrated UI areas easier to inspect.
    stem = path.stem
    for row in range(2):
        for col in range(4):
            box = (col * width // 4, row * height // 2, (col + 1) * width // 4, (row + 1) * height // 2)
            image.crop(box).save(out / f'{stem}-{row+1}-{col+1}.png', optimize=True)
    small = image.resize((180, 100))
    quantized = small.quantize(colors=8, method=Image.Quantize.MEDIANCUT).convert('RGB')
    palette = quantized.getpalette()[:24]
    colors[stem] = [f'#{palette[i]:02X}{palette[i+1]:02X}{palette[i+2]:02X}' for i in range(0, len(palette), 3)]
(root / 'docs' / 'design-colors.json').write_text(json.dumps(colors, indent=2) + '\n')
print(f'Sampled {len(boards)} boards and wrote {len(boards)*8} frame crops.')
