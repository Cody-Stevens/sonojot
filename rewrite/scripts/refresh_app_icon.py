"""Rebuild every app icon raster from the 1024px master inside icon.icns.

The supplied ICNS carries the approved SonoJot tile as its 1024px `ic10` PNG.
Every PNG and ICO in `src/brand/marks/app` is a plain resize of that master,
so no file can drift back to an older mark. Run from any directory:

    python rewrite/scripts/refresh_app_icon.py
"""
from pathlib import Path
from PIL import Image
import hashlib
import io
import json
import struct

root = Path(__file__).resolve().parents[1]
brand = root / "src/brand"
icons = brand / "marks/app"


def icns_master(path: Path) -> Image.Image:
    data = path.read_bytes()
    offset = 8
    while offset < len(data):
        kind = data[offset:offset + 4]
        length = struct.unpack(">I", data[offset + 4:offset + 8])[0]
        if kind == b"ic10":
            image = Image.open(io.BytesIO(data[offset + 8:offset + length])).convert("RGBA")
            if image.size != (1024, 1024):
                raise SystemExit(f"ic10 is {image.size}, expected 1024x1024")
            return image
        offset += length
    raise SystemExit("icon.icns has no 1024px ic10 entry")


master = icns_master(icons / "icon.icns")
outputs = {
    "icon.png": 512,
    "128x128@2x.png": 256,
    "128x128.png": 128,
    "64x64.png": 64,
    "32x32.png": 32,
}
written = []
for name, size in outputs.items():
    master.resize((size, size), Image.LANCZOS).save(icons / name, optimize=True)
    written.append(f"marks/app/{name}")
ico_sizes = [(n, n) for n in (16, 20, 24, 32, 40, 48, 64, 128, 256)]
for name in ("sonojot.ico", "icon.ico"):
    master.save(icons / name, format="ICO", sizes=ico_sizes)
    written.append(f"marks/app/{name}")

manifest_path = brand / "manifest.json"
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
present = {item["path"] for item in manifest["assets"]}
manifest["assets"] = [item for item in manifest["assets"] if (brand / item["path"]).is_file()]
for item in manifest["assets"]:
    data = (brand / item["path"]).read_bytes()
    item["bytes"] = len(data)
    item["sha256"] = hashlib.sha256(data).hexdigest()
for relative in written:
    if relative not in present:
        data = (brand / relative).read_bytes()
        manifest["assets"].append({"path": relative, "bytes": len(data),
                                   "sha256": hashlib.sha256(data).hexdigest()})
manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
print(json.dumps({"master": "marks/app/icon.icns#ic10", "written": written}))
