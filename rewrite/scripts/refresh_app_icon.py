"""Build the Windows container from the supplied SonoJot PNG, without redrawing it."""
from pathlib import Path
from PIL import Image
import hashlib
import json

root = Path(__file__).resolve().parents[1]
brand = root / "src/brand"
icons = brand / "marks/app"
evidence = root.parent / "artifacts/reliability-setup"
Image.open(icons / "icon.ico").convert("RGBA").save(evidence / "windows-icon-before.png")
source = Image.open(icons / "icon.png").convert("RGBA")
sizes = [(n, n) for n in (16, 20, 24, 32, 40, 48, 64, 128, 256)]
target = icons / "sonojot.ico"
source.save(target, format="ICO", sizes=sizes)
Image.open(target).convert("RGBA").save(evidence / "windows-icon-after.png")
manifest = json.loads((brand / "manifest.json").read_text())
relative = "marks/app/sonojot.ico"
manifest["assets"] = [item for item in manifest["assets"] if item["path"] != relative]
manifest["assets"].append({"path": relative, "bytes": target.stat().st_size,
                           "sha256": hashlib.sha256(target.read_bytes()).hexdigest()})
(brand / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(json.dumps({"source": "marks/app/icon.png", "output": relative, "sizes": sorted(Image.open(target).ico.sizes())}))
