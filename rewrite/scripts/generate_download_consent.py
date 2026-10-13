#!/usr/bin/env python3
"""Build the small, shared consent catalog from reviewed download evidence."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def generate():
    components = {c["id"]: c for c in json.loads(
        (ROOT / "licenses/download-components.json").read_text(encoding="utf-8"))["components"]}
    groups = []

    def add(group, ids, models, platform, label, summary, checkbox, links=()):
        records = [components[i] for i in ids]
        pins = [{"id": c["id"], "version": c["version"], "pins": c.get("artifactPins", [])}
                for c in records]
        version = hashlib.sha256(json.dumps(pins, sort_keys=True).encode()).hexdigest()
        destinations = [{"label": c["license"], "url": c["licenseUrl"]} for c in records]
        destinations.extend({"label": text, "url": url} for text, url in links)
        unique = {}
        for link in destinations:
            unique.setdefault(link["url"], link)
        groups.append(dict(id=group, version=version, models=models, platform=platform,
                           title=label, summary=summary, checkbox=checkbox,
                           components=[dict(name=c["name"], version=c["version"]) for c in records],
                           links=list(unique.values())))

    nvidia = "I agree to NVIDIA's license terms for these components"
    add("nvidia-granite-cuda12", ["nvidia-cuda-runtime-cu12", "nvidia-cublas-cu12"],
        ["granite-speech-4.1-2b-nar-q8"], "windows", "NVIDIA components for Granite",
        "Granite on Windows downloads CUDA runtime 12.8.90 and cuBLAS 12.8.4.1 from NVIDIA's packages on PyPI. Read the included package agreements in Credits and the versioned NVIDIA terms below.", nvidia,
        [("CUDA 12.8.1 redistributable components", "https://docs.nvidia.com/cuda/archive/12.8.1/eula/index.html#attachment-a")])
    add("nvidia-sherpa-cuda13", ["nvidia-cublas-13.7.0.27", "nvidia-cudnn-cu13-9.26.0.51"],
        ["qwen3-asr-1.7b-fp16-cuda", "whisper-tiny-fp32-cuda"], "windows", "NVIDIA CUDA and cuDNN",
        "These speech models use NVIDIA cuBLAS 13.7.0.27 and cuDNN 9.26.0.51. The captured package agreements differ from the versioned web documentation; both are retained in Credits. New sherpa CUDA runtime downloads are paused pending an ASR-only build.", nvidia,
        [("CUDA 13.3 redistributable components", "https://docs.nvidia.com/cuda/archive/13.3.0/eula/index.html#attachment-a"),
         ("cuDNN 9.26 redistribution and notices", "https://docs.nvidia.com/deeplearning/cudnn/backend/v9.26.0/reference/acknowledgements.html")])
    add("nvidia-prism-cuda13", ["nvidia-cuda13-prism-bundle"], ["ternary-bonsai-2-27b"], "windows",
        "NVIDIA components for Bonsai", "Bonsai's llama.cpp runtime downloads CUDA 13.3 libraries directly from the pinned PrismML release. NVIDIA's separate terms apply to those libraries.", nvidia,
        [("CUDA 13.3 redistributable components", "https://docs.nvidia.com/cuda/archive/13.3.0/eula/index.html#attachment-a")])
    add("nvidia-llama-cuda13", ["nvidia-cuda13-provisioned"],
        ["qwen3-asr-1.7b-q8-cuda", "nvidia-cuda13-provisioned"], "windows",
        "NVIDIA components for llama.cpp", "The Windows llama.cpp CUDA runtime uses NVIDIA CUDA 13.3 libraries from the pinned upstream release. Its files are never downloaded from SonoJot infrastructure.", nvidia,
        [("CUDA 13.3 redistributable components", "https://docs.nvidia.com/cuda/archive/13.3.0/eula/index.html#attachment-a")])
    for model in ["lfm2.5-1.2b", "lfm2.5-2.6b"]:
        add(model, [model], [model], "all", components[model]["name"],
            "Optional model under the LFM Open License v1.0. Commercial eligibility depends on your legal entity's annual revenue, including related entities, with a US$10 million threshold. The license uses different wording at exactly US$10 million; confirm eligibility with LiquidAI at or above that amount. A separate commercial license may be needed. Qwen3 is available under Apache-2.0.",
            "I have read the LFM license and confirm that I am eligible to use this model")
    for platform in ["windows", "macos"]:
        component = components["ffmpeg-" + platform]
        # Retired GPL packages remain in Credits, never in a new-download prompt.
        if component.get("automaticInstall") is False:
            continue
        sources = [("FFmpeg license and corresponding source information", "https://ffmpeg.org/legal.html")]
        for url in component["sources"]:
            if url.startswith("https://releases.sonojot.com/ffmpeg/") and url.endswith("/README.txt"):
                sources.append(("Exact source, build recipe and replacement instructions", url))
            if url.startswith("https://github.com/FFmpeg/FFmpeg/tree/"):
                sources.append(("Source for this FFmpeg revision", url))
            if url.startswith("https://github.com/BtbN/FFmpeg-Builds/blob/") and url.endswith("/README.md"):
                sources.append(("Build scripts and source information", url))
        add("ffmpeg-" + platform, ["ffmpeg-" + platform], ["ffmpeg"], platform,
            "Audio and video import", "Audio and video import uses FFmpeg (LGPL). Download it now? You can skip this and install later in Models → Performance and storage, or use your own FFmpeg.",
            "I acknowledge FFmpeg's license and source information",
            sources)
    return {"schemaVersion": 1, "groups": groups}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    output = ROOT / "legal/download-consent.json"
    content = json.dumps(generate(), ensure_ascii=False, indent=2) + "\n"
    if args.check:
        if not output.exists() or output.read_text(encoding="utf-8") != content:
            raise SystemExit("Download consent catalog is stale; run generate_download_consent.py")
    else:
        output.parent.mkdir(exist_ok=True)
        output.write_text(content, encoding="utf-8", newline="\n")
