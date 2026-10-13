#!/usr/bin/env python3
"""Generate the release license inventory without building or installing packages.

Run from any directory: python rewrite/scripts/generate_notices.py
Use --check for a read-only stale-output/coverage gate, --strict to also fail on
unresolved review issues, and --offline to forbid Cargo's registry network access.
Python 3.11+, Cargo, and the locked npm package sources are required to regenerate.
"""
from __future__ import annotations

import argparse
import base64
import collections
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tomllib
from typing import Any
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
TARGETS = ("x86_64-pc-windows-msvc", "aarch64-apple-darwin", "x86_64-apple-darwin")
LICENSE_NAME = re.compile(r"^(?:licen[cs]e|copying|copyright|notice|unlicen[cs]e|authors)(?:$|[._-])", re.I)
ASSET_EXTENSIONS = {".png", ".jpg", ".jpeg", ".svg", ".ico", ".icns", ".ttf", ".otf", ".woff", ".woff2", ".wav", ".mp3", ".ogg", ".flac", ".onnx", ".bin", ".dll", ".dylib", ".exe"}
NATIVE_EXTENSIONS = {".c", ".cc", ".cpp", ".cxx", ".h", ".hpp", ".s", ".asm", ".m", ".mm", ".metal"}
NON_RUNTIME_DIRS = {"test", "tests", "benches", "examples", "fuzz", ".git", "node_modules", "target"}


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def json_bytes(value: Any) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode("utf-8")


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def safe_relative(value: str) -> Path:
    # Reject Windows paths even on a Unix release runner.
    if not value or value.startswith("/") or "\\" in value or ":" in value:
        raise ValueError(f"Expected portable relative path: {value}")
    path = Path(value)
    if path.is_absolute() or ".." in path.parts:
        raise ValueError(f"Unsafe relative path: {value}")
    return path


def package_id(package: dict) -> str:
    return f"rust:{package['name']}@{package['version']}"


def verify_published_crate(directory: Path, archive: bytes, checksum: str, prefix: str) -> dict:
    """Compare the resolved source tree with every file in a locked .crate archive."""
    if sha(archive) != checksum:
        raise ValueError(f"Published crate checksum differs from Cargo.lock: {prefix}")
    hashes = {}
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as source:
        for member in source:
            if member.isdir():
                continue
            if not member.isfile() or not member.name.startswith(prefix + "/"):
                raise ValueError(f"Unexpected published crate member: {member.name}")
            relative = safe_relative(member.name[len(prefix) + 1:])
            if relative.as_posix() in hashes:
                raise ValueError(f"Duplicate published crate member: {member.name}")
            data = source.extractfile(member).read()
            path = directory / relative
            if path.is_symlink() or not path.resolve().is_relative_to(directory.resolve()):
                raise ValueError(f"Redirected MPL source file: {prefix}/{relative}")
            if not path.is_file() or path.read_bytes() != data:
                raise ValueError(f"Modified or missing MPL source file: {prefix}/{relative}")
            hashes[relative.as_posix()] = sha(data)
    if not hashes:
        raise ValueError(f"Empty published crate archive: {prefix}")
    generated_metadata = {".cargo-ok", ".cargo-checksum.json"}
    extras = sorted(p.relative_to(directory).as_posix() for p in directory.rglob("*")
                    if p.is_file() and p.relative_to(directory).as_posix() not in hashes
                    and p.relative_to(directory).as_posix() not in generated_metadata)
    if extras:
        raise ValueError(f"Extra files in MPL source tree: {prefix}: {', '.join(extras)}")
    return {"unmodified": True, "verifiedFileCount": len(hashes), "fileSha256": dict(sorted(hashes.items()))}


def cargo_closures(metadata: dict) -> tuple[set[str], set[str]]:
    """Follow normal edges only; stop at proc macros, including their helpers.

    A package reached both through normal and build edges remains a runtime
    package. Build dependencies and proc macros have a separate non-dev closure.
    """
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    root = metadata["resolve"]["root"]
    if not root or root not in nodes:
        raise ValueError("Cargo metadata has no resolved application root")
    runtime: set[str] = set()
    all_build: set[str] = set()

    def visit(start: str, seen: set[str], normal_only: bool) -> None:
        pending = [start]
        while pending:
            current = pending.pop()
            if current in seen:
                continue
            seen.add(current)
            for dep in nodes[current]["deps"]:
                kinds = {k["kind"] for k in dep["dep_kinds"]}
                if not (None in kinds if normal_only else kinds & {None, "build"}):
                    continue
                target = packages[dep["pkg"]]
                if normal_only and any("proc-macro" in t["kind"] for t in target["targets"]):
                    continue
                pending.append(dep["pkg"])

    visit(root, runtime, True)
    visit(root, all_build, False)
    runtime.discard(root)
    all_build.discard(root)
    return runtime, all_build - runtime


def production_npm(lock: dict) -> list[tuple[str, dict]]:
    if lock.get("lockfileVersion", 0) < 2 or "packages" not in lock:
        raise ValueError("An npm lockfile with a packages table is required")
    # devOptional also belongs to an optional production branch, so retain it.
    return sorted((path, data) for path, data in lock["packages"].items()
                  if path and not data.get("dev"))


def license_files(directory: Path) -> list[Path]:
    result = []
    for path in directory.rglob("*"):
        relative = path.relative_to(directory)
        if set(relative.parts[:-1]) & NON_RUNTIME_DIRS:
            continue
        if path.is_file() and LICENSE_NAME.match(path.name) and path.suffix.lower() not in {".rs", ".h", ".nsh", ".html", ".json"}:
            result.append(path)
    return sorted(result)


def notice_comments(data: bytes) -> list[tuple[int, int, bytes]]:
    """Keep complete comment blocks containing attribution, without rewriting."""
    pattern = rb"/\*.*?\*/|(?:(?:^[ \t]*(?://|\# |;)[^\r\n]*(?:\r?\n|$))+)"
    return [(m.start(), m.end(), m[0]) for m in re.finditer(pattern, data, re.M | re.S)
            if re.search(rb"copyright|SPDX-License-Identifier|redistribution and use", m[0], re.I)]


class Generator:
    def __init__(self, root: Path = ROOT, check: bool = False, offline: bool = False):
        self.root = root
        self.licenses = root / "licenses"
        self.check = check
        self.offline = offline
        self.outputs: dict[str, bytes] = {}
        self.inputs: dict[str, str] = {}
        self.components: list[dict] = []
        self.build_tools: list[dict] = []
        self.errors: list[str] = []
        self.warnings: list[str] = []
        self.texts: dict[str, dict] = {}
        self.packages: dict[str, dict] = {}
        self.graphs: list[dict] = []
        self.asset_paths: set[str] = set()

    def input(self, path: Path) -> bytes:
        data = path.read_bytes()
        relative = path.relative_to(self.root).as_posix()
        # Hash source text as Git stores it, so CRLF (Windows) and LF checkouts
        # agree. licenses/ is -text and binary files are never converted.
        text = not relative.startswith("licenses/") and b"\x00" not in data[:8000]
        self.inputs[relative] = sha(data.replace(b"\r\n", b"\n") if text else data)
        return data

    def emit(self, relative: str, data: bytes) -> None:
        self.outputs[relative] = data

    def text(self, data: bytes, source: str, original_path: str, role: str = "license", encoding: str = "utf-8") -> dict:
        # Preserve original bytes; Markdown only adds an outer fence.
        if not data.strip():
            self.errors.append(f"Empty license/notice text: {source}")
            return {}
        try:
            data.decode(encoding)
        except UnicodeDecodeError:
            self.errors.append(f"Non-UTF-8 notice requires an explicit encoding review: {source}")
            return {}
        digest = sha(data)
        path = f"licenses/texts/{digest}.txt"
        self.emit(path, data)
        record = {"sha256": digest, "path": path, "bytes": len(data), "encoding": encoding}
        self.texts[digest] = record
        return {**record, "source": source, "originalPath": original_path, "role": role}

    def collect(self, directory: Path, source_base: str, paths: list[Path] | None = None) -> list[dict]:
        texts = []
        for path in license_files(directory) if paths is None else paths:
            relative = path.relative_to(directory).as_posix()
            role = "attribution" if path.name.lower().startswith(("copyright", "notice", "authors")) else "license"
            item = self.text(path.read_bytes(), source_base + quote(relative, safe="/"), relative, role)
            if item:
                texts.append(item)
        return texts

    def cargo(self) -> None:
        lock = tomllib.loads(self.input(self.root / "src-tauri/Cargo.lock").decode())
        checksums = {(p["name"], p["version"]): p.get("checksum") for p in lock["package"]}
        self.input(self.root / "src-tauri/Cargo.toml")
        collected: dict[str, dict] = {}
        for target in TARGETS:
            print(f"Resolving locked Cargo graph: {target}", file=sys.stderr, flush=True)
            command = ["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target]
            if self.offline:
                command.append("--offline")
            process = subprocess.run(command, cwd=self.root / "src-tauri", capture_output=True, check=True, timeout=300)
            metadata = json.loads(process.stdout)
            runtime, build = cargo_closures(metadata)
            packages = {p["id"]: p for p in metadata["packages"]}
            nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
            snapshot = {"target": target, "command": [c for c in command if c != "--offline"], "runtime": sorted(package_id(packages[p]) for p in runtime),
                        "buildOnly": sorted(package_id(packages[p]) for p in build), "packages": []}
            for key in sorted(runtime | build):
                package = packages[key]
                identifier = package_id(package)
                if identifier in self.packages and self.packages[identifier]["source"] != package["source"]:
                    raise ValueError(f"Ambiguous package source: {identifier}")
                self.packages[identifier] = package
                record = collected.setdefault(identifier, {
                    "id": identifier, "kind": "rust", "name": package["name"], "version": package["version"],
                    "license": package.get("license") or "UNKNOWN", "repository": package.get("repository"),
                    "sources": [f"https://crates.io/crates/{package['name']}/{package['version']}",
                                f"https://docs.rs/crate/{package['name']}/{package['version']}/source/Cargo.toml"],
                    "checksum": checksums.get((package["name"], package["version"])),
                    "targets": [], "buildTargets": [], "features": {}, "issues": [], "texts": [],
                })
                record["targets" if key in runtime else "buildTargets"].append(target)
                record["features"][target] = sorted(nodes[key]["features"])
                snapshot["packages"].append({"id": identifier, "license": record["license"], "repository": record["repository"],
                    "checksum": record["checksum"], "features": record["features"][target],
                    "dependencies": sorted([{"id": package_id(packages[d["pkg"]]), "kinds": d["dep_kinds"]}
                                            for d in nodes[key]["deps"]], key=lambda d: d["id"])})
            snapshot["packages"].sort(key=lambda p: p["id"])
            self.emit(f"licenses/evidence/cargo-{target}.json", json_bytes(snapshot))
            self.graphs.append({"target": target, "runtimeCount": len(runtime), "buildOnlyCount": len(build),
                                "evidence": f"licenses/evidence/cargo-{target}.json"})
        for identifier, component in sorted(collected.items()):
            component["targets"].sort()
            component["buildTargets"].sort()
            if not component["targets"]:
                component["distribution"] = "build tooling only; excluded from bundled notices"
                self.build_tools.append(component)
                continue
            package = self.packages[identifier]
            directory = Path(package["manifest_path"]).parent
            base = f"https://docs.rs/crate/{package['name']}/{package['version']}/source/"
            component["texts"] = self.collect(directory, base)
            if package.get("license_file"):
                path = directory / package["license_file"]
                if path.is_file() and not any(t["originalPath"] == path.relative_to(directory).as_posix() for t in component["texts"]):
                    component["texts"] += self.collect(directory, base, [path])
            component["distribution"] = "normal dependency closure; bundled application"
            component["manifestSha256"] = sha(Path(package["manifest_path"]).read_bytes())
            vcs = directory / ".cargo_vcs_info.json"
            if vcs.exists():
                component["upstreamRevision"] = read_json(vcs)
            self.components.append(component)

    def npm(self) -> None:
        lock = json.loads(self.input(self.root / "package-lock.json"))
        self.input(self.root / "package.json")
        collected: dict[str, dict] = {}
        for location, package in production_npm(lock):
            directory = self.root / safe_relative(location)
            name = location.rsplit("node_modules/", 1)[-1]
            component = {"id": f"npm:{name}@{package['version']}", "kind": "npm", "name": name,
                         "version": package["version"], "license": package.get("license") or "UNKNOWN",
                         "lockPaths": [location], "integrity": package.get("integrity"),
                         "sources": [package.get("resolved", f"https://www.npmjs.com/package/{name}/v/{package['version']}")],
                         "distribution": "production npm lock closure; conservative bundle inventory (may include compiler helpers)",
                         "issues": [], "texts": []}
            if not (directory / "package.json").is_file():
                self.errors.append(f"Locked npm package source is absent: {component['id']}")
            else:
                manifest = read_json(directory / "package.json")
                if manifest.get("version") != package["version"] or manifest.get("name") != name:
                    self.errors.append(f"Installed npm package differs from lock: {component['id']}")
                component["repository"] = manifest.get("repository")
                component["texts"] = self.collect(directory, f"https://unpkg.com/{name}@{package['version']}/")
                component["manifestSha256"] = sha((directory / "package.json").read_bytes())
            previous = collected.get(component["id"])
            if previous:
                if any(previous.get(field) != component.get(field) for field in ("integrity", "license", "manifestSha256")):
                    self.errors.append(f"Conflicting npm sources for the same package version: {component['id']}")
                previous["lockPaths"].append(location)
                previous["texts"].extend(component["texts"])
            else:
                collected[component["id"]] = component
        self.components.extend(collected.values())

    def supplements(self) -> None:
        path = self.licenses / "supplements.json"
        if not path.exists():
            self.errors.append("Missing licenses/supplements.json (native and installer evidence)")
            return
        data = json.loads(self.input(path))
        by_id = {c["id"]: c for c in self.components}
        for addition in data.get("components", []):
            component = dict(addition)
            component.setdefault("issues", [])
            component.setdefault("texts", [])
            parent_id = component.get("parent")
            if parent_id:
                if parent_id not in by_id:
                    self.errors.append(f"Supplement parent is no longer in normal closure: {parent_id}")
                    continue
                if "targets" in by_id[parent_id]:
                    component["targets"] = by_id[parent_id]["targets"]
            self.components.append(component)
            by_id[component["id"]] = component
        for identifier, supplement in data.get("overrides", {}).items():
            if identifier not in by_id:
                self.errors.append(f"Stale license supplement: {identifier}")
                continue
            component = by_id[identifier]
            for key, value in supplement.items():
                if key not in {"evidence", "issues"}:
                    component[key] = value
            component["issues"].extend(supplement.get("issues", []))
            component.setdefault("evidence", []).extend(supplement.get("evidence", []))
        for component in self.components:
            source_notices = component.pop("sourceNotices", [])
            if source_notices:
                directory = Path(self.packages[component["id"]]["manifest_path"]).parent
                for pattern in source_notices:
                    for path in sorted(directory.glob(pattern)):
                        if not path.is_file():
                            continue
                        source_data = path.read_bytes()
                        relative = path.relative_to(directory).as_posix()
                        source = f"https://docs.rs/crate/{component['name']}/{component['version']}/source/{quote(relative, safe='/')}"
                        for start, end, comment in notice_comments(source_data):
                            item = self.text(comment, source, relative, "source-attribution")
                            if item:
                                item.update({"sourceSha256": sha(source_data), "byteStart": start, "byteEnd": end})
                                component["texts"].append(item)
            for evidence in component.pop("evidence", []):
                data_bytes = None
                if "cratePath" in evidence:
                    owner = evidence.get("crate", component.get("parent", component["id"]))
                    package = self.packages.get(owner)
                    if not package:
                        self.errors.append(f"Missing source package for evidence: {owner}")
                        continue
                    path = Path(package["manifest_path"]).parent / safe_relative(evidence["cratePath"])
                    if path.exists():
                        data_bytes = path.read_bytes()
                else:
                    path = self.root / safe_relative(evidence["path"])
                    if path.exists():
                        data_bytes = self.input(path)
                if data_bytes is None:
                    self.errors.append(f"Missing supplemental text: {component['id']}: {evidence}")
                    continue
                if evidence.get("sha256") and sha(data_bytes) != evidence["sha256"]:
                    self.errors.append(f"Supplemental evidence changed: {component['id']}: {evidence['source']}")
                item = self.text(data_bytes, evidence["source"], evidence.get("cratePath", evidence.get("path", "")), evidence.get("role", "license"))
                if item:
                    component["texts"].append(item)

    def upstream_evidence(self) -> None:
        path = self.licenses / "upstream-evidence.json"
        if not path.is_file():
            self.errors.append("Missing licenses/upstream-evidence.json")
            return
        by_id = {c["id"]: c for c in self.components}
        for record in json.loads(self.input(path))["records"]:
            data = base64.b64decode(record["base64"], validate=True)
            if sha(data) != record["sha256"]:
                raise ValueError(f"Upstream evidence hash mismatch: {record['url']}")
            for identifier in record["ids"]:
                if identifier not in by_id:
                    self.errors.append(f"Stale upstream evidence: {identifier}")
                    continue
                component = by_id[identifier]
                item = self.text(data, record["url"], record.get("originalPath", record["url"].rsplit("/", 1)[-1]), record.get("role", "license"), record.get("encoding", "utf-8"))
                if item:
                    component["texts"].append(item)
                    component["sources"].append(record["url"])
                    component.setdefault("upstreamEvidence", []).append({k: v for k, v in record.items() if k not in {"ids", "base64", "issues"}})
                if record.get("role", "license") == "license":
                    component["wrapperLicenseVerified"] = True
                component["issues"].extend(record.get("issues", []))

    def native_audit(self) -> None:
        rules = json.loads(self.input(self.licenses / "native-rules.json"))["crates"]
        report = []
        for component in self.components:
            if component["kind"] != "rust":
                continue
            directory = Path(self.packages[component["id"]]["manifest_path"]).parent
            native_files = [p for p in directory.rglob("*") if p.is_file() and
                            p.suffix.lower() in NATIVE_EXTENSIONS | {".a", ".lib", ".dll", ".dylib"} and
                            not set(p.relative_to(directory).parts[:-1]) & NON_RUNTIME_DIRS]
            if not native_files:
                continue
            rule = rules.get(component["id"])
            if not rule:
                self.errors.append(f"Native source/binary has no reviewed coverage rule: {component['id']}")
                rule = {"include": [], "reason": "Unreviewed native source or binary"}
            for feature in rule.get("forbiddenFeatures", []):
                if any(feature in features for features in component["features"].values()):
                    self.errors.append(f"Native coverage rule invalidated by feature {feature}: {component['id']}")
            included = set()
            for pattern in rule["include"]:
                included.update(p for p in directory.glob(pattern) if p.is_file())
            files = []
            for path in sorted(set(native_files) | included):
                relative = path.relative_to(directory).as_posix()
                data = path.read_bytes()
                files.append({"path": relative, "sha256": sha(data), "covered": path in included})
                if path not in included:
                    continue
                if path.suffix.lower() not in NATIVE_EXTENSIONS | {".rs", ".pl"}:
                    continue
                source = f"https://docs.rs/crate/{component['name']}/{component['version']}/source/{quote(relative, safe='/')}"
                for start, end, text in notice_comments(data):
                    item = self.text(text, source, relative, "source-attribution")
                    if item:
                        item.update({"sourceSha256": sha(data), "byteStart": start, "byteEnd": end})
                        component["texts"].append(item)
            report.append({"component": component["id"], "reason": rule["reason"], "files": files})
        for identifier in set(rules) - {c["id"] for c in self.components}:
            self.errors.append(f"Stale native coverage rule: {identifier}")
        self.emit("licenses/evidence/native-coverage.json", json_bytes(report))

    def assets(self) -> None:
        manifest = json.loads(self.input(self.licenses / "assets.json"))
        covered: set[str] = set()
        for entry in manifest["components"]:
            component = {**entry, "texts": [], "files": [], "issues": list(entry.get("issues", []))}
            for pattern in component.pop("patterns", []):
                paths = sorted(p for p in self.root.glob(pattern) if p.is_file())
                if not paths:
                    self.errors.append(f"Asset rule has no matches: {pattern}")
                for path in paths:
                    relative = path.relative_to(self.root).as_posix()
                    covered.add(relative)
                    self.input(path)
                    component["files"].append({"path": relative, "sha256": self.inputs[relative]})
            for relative in component.pop("textFiles", []):
                path = self.root / safe_relative(relative)
                component["texts"].append(self.text(self.input(path), relative, relative))
            for relative in component.get("provenanceFiles", []):
                self.input(self.root / safe_relative(relative))
            component["files"] = sorted({f["path"]: f for f in component["files"]}.values(), key=lambda f: f["path"])
            self.components.append(component)
        candidates = set()
        for base in ("src", "public", "src-tauri/assets", "src-tauri/icons", "src-tauri/resources"):
            for path in (self.root / base).rglob("*"):
                whole_directory = base in {"public", "src-tauri/assets", "src-tauri/resources"}
                if path.is_file() and (whole_directory or path.suffix.lower() in ASSET_EXTENSIONS) and "tests" not in path.relative_to(self.root).parts:
                    candidates.add(path.relative_to(self.root).as_posix())
        config = json.loads(self.input(self.root / "src-tauri/tauri.conf.json"))
        for relative in ("src-tauri/build.rs", "src-tauri/windows/installer.nsi", "src-tauri/windows/brand-hooks.nsh", "src-tauri/windows/README.md", "vite.config.ts", "index.html"):
            self.input(self.root / relative)
        bundle = config.get("bundle", {})
        resources = bundle.get("resources", {})
        resource_patterns = list(resources) + bundle.get("icon", []) + bundle.get("externalBin", [])
        for pattern in resource_patterns:
            # Tauri paths are relative to src-tauri. Notices are this generator's output.
            paths = sorted((self.root / "src-tauri").glob(pattern))
            if "THIRD_PARTY_NOTICES.md" in pattern:
                continue
            if not paths:
                self.errors.append(f"Unresolved Tauri resource pattern: {pattern}")
            for path in paths:
                children = sorted(p for p in path.rglob("*") if p.is_file()) if path.is_dir() else [path]
                for child in children:
                    try:
                        candidates.add(child.resolve().relative_to(self.root.resolve()).as_posix())
                    except ValueError:
                        self.errors.append(f"Tauri resource is outside rewrite and needs a rule: {pattern}")
        # Literal include macros catch non-media inputs such as dictionaries.
        # Ignore known test-only directories; remaining includes are conservative.
        # Only the include paths matter (asset-coverage.json); hashing every Rust
        # file would make the inventory stale on unrelated code changes.
        for path in sorted((self.root / "src-tauri/src").rglob("*.rs")):
            source = path.read_bytes().decode("utf-8")
            for match in re.finditer(r'include_(?:bytes|str)!\(\s*"([^"\n]+)"', source):
                relative_path = (path.parent / match[1]).resolve()
                try:
                    relative = relative_path.relative_to(self.root.resolve()).as_posix()
                except ValueError:
                    self.errors.append(f"Embedded file is outside rewrite: {path.name}: {match[1]}")
                    continue
                if relative.startswith("evaluation/") or path.name == "tests.rs" or relative.endswith(("_cases.json", "tauri.conf.json")):
                    continue
                if relative.endswith("THIRD_PARTY_NOTICES.md"):
                    continue
                if relative in {"licenses/download-components.json", "licenses/model-links.json"}:
                    continue  # Validated and hashed by downloads().
                candidates.add(relative)
        self.asset_paths = candidates
        for relative in sorted(candidates - covered):
            self.errors.append(f"Bundled asset has no provenance/license rule: {relative}")
        self.emit("licenses/evidence/asset-coverage.json", json_bytes({"candidates": sorted(candidates), "covered": sorted(covered),
            "scope": "All source media, public files, Tauri resources/icons, and literal production Rust includes; unused source artwork is retained conservatively."}))

    def download_evidence(self, manifest: str) -> dict[str, tuple[bytes, str]]:
        """Validate captured bytes and recover originals before rendering notices."""
        evidence = json.loads(self.input(self.licenses / safe_relative(manifest)))
        records = {entry["path"]: entry for entry in evidence["files"]}
        if len(records) != len(evidence["files"]):
            raise ValueError("Duplicate download evidence path")
        result = {}
        for relative, entry in records.items():
            path = self.licenses / safe_relative(relative)
            if not path.resolve().is_relative_to(self.licenses.resolve()):
                raise ValueError(f"Download evidence path escapes licenses directory: {relative}")
            saved = self.input(path)
            if sha(saved) != entry["savedSha256"] or len(saved) != entry["savedBytes"]:
                raise ValueError(f"Download evidence saved hash/size mismatch: {relative}")
            original = saved
            if "originalBytesBase64" in entry:
                original = base64.b64decode(entry["originalBytesBase64"], validate=True)
            if original.replace(b"\r\n", b"\n").rstrip(b"\n") != saved.replace(b"\r\n", b"\n").rstrip(b"\n"):
                raise ValueError(f"Download legal text differs beyond newline normalization: {relative}")
            if "sourceSha256" in entry and (sha(original) != entry["sourceSha256"] or len(original) != entry["sourceBytes"]):
                raise ValueError(f"Download evidence source hash/size mismatch: {relative}")
            source = entry.get("url")
            if not source and entry.get("derivedFrom"):
                source = records[entry["derivedFrom"]].get("url")
            result[relative] = (original, source or f"licenses/{relative}")
        return result

    def downloads(self) -> tuple[list[dict], list[dict], dict]:
        path = self.licenses / "download-components.json"
        if not path.exists():
            self.errors.append("Missing mandatory licenses/download-components.json; optional-download coverage is incomplete")
            return [], [], {}
        data = json.loads(self.input(path))
        if data.get("schemaVersion") != 1:
            self.errors.append("Unsupported download-components.json schemaVersion")
        evidence = self.download_evidence(data["evidenceManifest"]) if data.get("evidenceManifest") else {}
        components = []
        identifiers = set()
        for entry in data.get("components", []):
            if entry.get("kind") not in {"model", "runtime"}:
                self.errors.append(f"Invalid download component kind: {entry.get('id', '(missing id)')}")
            if entry["id"] in identifiers:
                self.errors.append(f"Duplicate download component: {entry['id']}")
            identifiers.add(entry["id"])
            if entry.get("license", "").strip().lower() in {"", "unknown", "unverified", "noassertion", "pending"}:
                self.errors.append(f"Unverified download license: {entry['id']}")
            # Keep every supplier field unchanged, including kind and textFiles.
            component = {**entry, "noticeTexts": []}
            for relative in entry.get("textFiles", []):
                path = self.licenses / safe_relative(relative)
                if not path.resolve().is_relative_to(self.licenses.resolve()):
                    raise ValueError(f"Download license path escapes licenses directory: {relative}")
                if not path.is_file():
                    self.errors.append(f"Missing download license text: {entry['id']}: {relative}")
                    continue
                if data.get("evidenceManifest") and relative not in evidence:
                    self.errors.append(f"Download notice has no captured evidence record: {relative}")
                raw, source = evidence.get(relative, (self.input(path), entry.get("licenseUrl", "")))
                item = self.text(raw, source, f"licenses/{relative}")
                if item:
                    component["noticeTexts"].append(item)
            if not component["noticeTexts"]:
                self.errors.append(f"No license/notice text supplied for download: {entry['id']}")
            if entry.get("issues"):
                self.warnings += [f"{entry['id']}: {issue}" for issue in entry["issues"]]
            components.append(component)
        if not components:
            self.errors.append("Optional-download inventory is empty")
        return components, data.get("services", []), {k: v for k, v in data.items() if k not in {"components", "services"}}

    def validate(self) -> None:
        ids = set()
        for component in self.components:
            identifier = component["id"]
            if identifier in ids:
                self.errors.append(f"Duplicate component: {identifier}")
            ids.add(identifier)
            component["texts"] = sorted({(t["sha256"], t["source"]): t for t in component["texts"] if t}.values(),
                                         key=lambda t: (t["source"], t["sha256"]))
            full_texts = [t for t in component["texts"] if t["role"] in {"license", "standard-license"} and
                          not self.outputs[t["path"]].lstrip().startswith(b"SPDXVersion:")]
            if component.get("requiresNotice", True) and not full_texts:
                self.errors.append(f"Missing full license text: {identifier}")
            if component.get("license") in {None, "UNKNOWN", "NOASSERTION"}:
                self.errors.append(f"Unverified license: {identifier}")
            if component["kind"] == "rust" and full_texts and all(t["originalPath"].count("/") for t in full_texts) and not component.get("wrapperLicenseVerified"):
                self.errors.append(f"Only nested licenses found; wrapper license needs evidence: {identifier}")
            self.warnings += [f"{identifier}: {issue}" for issue in component.get("issues", [])]
            if component.get("license") == "MPL-2.0" and not component.get("sourceAvailability", {}).get("unmodified"):
                self.warnings.append(f"{identifier}: MPL-2.0 covered source must remain available; retain the exact upstream source and identify any modifications.")

    def mpl_sources(self) -> None:
        verified = []
        for component in self.components:
            if component["kind"] != "rust" or component.get("license") != "MPL-2.0":
                continue
            identifier = component["id"]
            package = self.packages[identifier]
            name, version = component["name"], component["version"]
            archive = f"sources/{name}-{version}.crate"
            if package.get("source") != "registry+https://github.com/rust-lang/crates.io-index":
                self.errors.append(f"MPL component is patched or has a non-crates.io source: {identifier}")
                continue
            try:
                result = verify_published_crate(Path(package["manifest_path"]).parent,
                    self.input(self.licenses / archive), component["checksum"], f"{name}-{version}")
            except (ValueError, OSError, tarfile.TarError) as error:
                self.errors.append(str(error))
                continue
            component["sourceAvailability"] = {
                "license": "MPL-2.0", "unmodified": True,
                "versionUrl": f"https://crates.io/crates/{name}/{version}",
                "sourceUrl": f"https://crates.io/api/v1/crates/{name}/{version}/download",
                "retainedArchive": f"licenses/{archive}", "archiveSha256": component["checksum"],
                "verifiedFileCount": result["verifiedFileCount"],
                "verification": "Resolved Cargo registry source matches every published archive file byte-for-byte; no extra source files. Cargo source overrides are rejected.",
            }
            verified.append({"id": identifier, **component["sourceAvailability"], "fileSha256": result["fileSha256"]})
        self.emit("licenses/evidence/mpl-sources.json", json_bytes({"schemaVersion": 1, "components": verified}))

    def render(self, inventory: dict) -> bytes:
        lines = ["# SonoJot third-party notices", "", "Generated by `scripts/generate_notices.py`. Do not edit this file by hand.", "",
                 "This document covers the locked normal Rust dependency closure for Windows x64 and macOS ARM64/x64, production npm dependencies, bundled assets, installer components, and optional downloads. Build-only crates and proc macros are recorded separately in `licenses/inventory.json`. A dependency inventory may include code removed by the linker or frontend bundler.", "",
                 "License declarations and source links are recorded per component. Original license, copyright and NOTICE bytes are preserved in `licenses/texts/`; identical bytes are printed once below. A license expression with alternatives does not require using every alternative. Nested third-party terms are retained separately from wrapper declarations.", ""]
        if inventory["validation"]["errors"]:
            lines += ["## Incomplete coverage", "", "The following evidence must be supplied or reviewed before release:", ""]
            lines += [f"- {issue}" for issue in inventory["validation"]["errors"]]
            lines.append("")
        mpl = [c for c in self.components if c.get("sourceAvailability")]
        if mpl:
            lines += ["## Source for MPL-2.0 components", "",
                      "The following components use unmodified published source under the Mozilla Public License 2.0. Obtain each exact version's source from the crates.io links below. SonoJot's release source also retains the original `.crate` archives under `rewrite/licenses/sources/`. The notices generator compares the resolved source files byte-for-byte with those archives and checks their SHA-256 against Cargo.lock.", ""]
            for component in sorted(mpl, key=lambda c: c["name"]):
                source = component["sourceAvailability"]
                lines.append(f"- [{component['name']} {component['version']}]({source['versionUrl']}): [download source]({source['sourceUrl']}); unmodified, {source['verifiedFileCount']} files verified; archive SHA-256 `{source['archiveSha256']}`.")
            lines += ["", "The MPL-2.0 terms are included in the license texts below. Recipients retain the rights granted to this source by MPL-2.0.", ""]
        groups = [("Rust runtime dependencies", [c for c in self.components if c["kind"] == "rust"]),
                  ("Production npm dependencies", [c for c in self.components if c["kind"] == "npm"]),
                  ("Native libraries, assets and installer components", [c for c in self.components if c["kind"] not in {"rust", "npm"}]),
                  ("Optional downloads", inventory["downloads"])]
        usage: dict[str, list[str]] = collections.defaultdict(list)
        for title, components in groups:
            lines += [f"## {title}", ""]
            if not components:
                lines += ["No component evidence supplied.", ""]
            for component in sorted(components, key=lambda c: c["id"]):
                lines += [f"### {component['name']} {component.get('version', '')}".rstrip(), "",
                          f"- Inventory ID: `{component['id']}`", f"- License: `{component.get('license', 'UNKNOWN')}`"]
                if component.get("targets"):
                    lines.append("- Targets: " + ", ".join(component["targets"]))
                for source in component.get("sources", []):
                    lines.append(f"- Source: {source}")
                if component.get("licenseUrl"):
                    lines.append(f"- License details: {component['licenseUrl']}")
                for field in ("distribution", "attribution", "conditions", "verification"):
                    if component.get(field):
                        lines.append(f"- {field.capitalize()}: {component[field]}")
                for issue in component.get("issues", []):
                    lines.append(f"- Review: {issue}")
                component_texts = component.get("noticeTexts", component.get("texts", []))
                for text in component_texts:
                    digest = text["sha256"]
                    label = "Standard license text (see attribution review)" if text["role"] == "standard-license" else "Text"
                    lines.append(f"- {label}: [{text['originalPath']}](#text-{digest}) (SHA-256 `{digest}`)")
                    usage[digest].append(f"{component['id']} — {text['originalPath']} — {text['source']}")
                lines.append("")
        lines += ["## License and attribution texts", ""]
        # Compose bytes so CRLFs, trailing whitespace, and final-newline absence
        # in the original text cannot be normalized by Python's text I/O.
        output = ("\n".join(lines) + "\n").encode("utf-8")
        for digest in sorted(usage):
            raw = self.outputs[self.texts[digest]["path"]]
            data = raw.decode(self.texts[digest]["encoding"]).encode("utf-8")
            longest = max((len(m[0]) for m in re.finditer(rb"`+", data)), default=0)
            fence = b"`" * max(3, longest + 1)
            heading = f'<a id="text-{digest}"></a>\n\n### Text {digest}\n\nUsed by:\n\n'
            heading += "\n".join(f"- {line}" for line in sorted(set(usage[digest]))) + "\n\n"
            output += heading.encode("utf-8") + fence + b"text\n" + data + (b"" if data.endswith(b"\n") else b"\n") + fence + b"\n\n"
        return output

    def run(self) -> dict:
        self.input(Path(__file__))
        self.cargo()
        self.npm()
        self.supplements()
        self.upstream_evidence()
        self.mpl_sources()
        self.native_audit()
        self.assets()
        downloads, services, download_metadata = self.downloads()
        # Ship only the fields needed by the library and OS link opener.
        self.emit("licenses/model-links.json", json_bytes({"components": [
            {key: entry[key] for key in ("kind", "license", "licenseUrl", "catalogIds")}
            for entry in sorted(downloads, key=lambda item: item["id"]) if entry["kind"] == "model"
        ]}))
        self.validate()
        counts = dict(sorted(collections.Counter(c["kind"] for c in self.components).items()))
        counts.update({"buildOnlyRust": len(self.build_tools), "models": sum(c.get("kind") == "model" for c in downloads),
                       "runtimes": sum(c.get("kind") == "runtime" for c in downloads), "uniqueTexts": len(self.texts), "assetFiles": len(self.asset_paths)})
        inventory = {"schemaVersion": 1, "targets": list(TARGETS), "counts": counts,
                     "methodology": {
                         "rust": "cargo metadata --locked --format-version 1 --filter-platform TARGET; normal edges only; proc macros and their exclusive dependencies excluded; Cargo feature unification may conservatively retain extra runtime nodes.",
                         "npm": "package-lock.json packages entries excluding dev=true; retain devOptional (also optional production dependencies). Production closure includes Svelte compiler helpers even when tree-shaken. https://docs.npmjs.com/cli/v11/configuring-npm/package-lock-json/#packages",
                         "native": "Pinned manual supplements plus recursive license-file collection; review native source and toolchain changes before release.",
                         "assets": "Rules in licenses/assets.json cover source media, Tauri resources/icons and literal Rust embedded inputs; file hashes identify exact bytes.",
                         "limits": "This is a source inventory, not a scan of a built installer. OS-provided frameworks and dynamic vendor runtimes are identified separately. Compiler runtime and installer versions must be checked on the release machine."},
                     "inputs": dict(sorted(self.inputs.items())), "cargoGraphs": self.graphs,
                     "components": sorted(self.components, key=lambda c: c["id"]),
                     "buildTools": sorted(self.build_tools, key=lambda c: c["id"]),
                     "downloads": sorted(downloads, key=lambda c: c["id"]), "downloadMetadata": download_metadata,
                     "services": services, "texts": sorted(self.texts.values(), key=lambda t: t["sha256"]),
                     "validation": {"errors": sorted(set(self.errors)), "reviewIssues": sorted(set(self.warnings))}}
        self.emit("THIRD_PARTY_NOTICES.md", self.render(inventory))
        self.emit("licenses/inventory.json", json_bytes(inventory))
        return inventory

    def write_or_check(self) -> list[str]:
        stale = []
        # This directory contains only content-addressed generator outputs.
        # Retire old texts when a dependency or captured source changes.
        for path in sorted((self.licenses / "texts").glob("*.txt")):
            relative = path.relative_to(self.root).as_posix()
            if relative not in self.outputs:
                stale.append(relative)
                if not self.check:
                    path.unlink()
        for relative, data in sorted(self.outputs.items()):
            path = self.root / relative
            if not path.is_file() or path.read_bytes() != data:
                stale.append(relative)
                if not self.check:
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(data)
        return stale


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Read-only gate: fail for stale outputs or missing mandatory evidence")
    parser.add_argument("--strict", action="store_true", help="Also fail for unresolved review issues")
    parser.add_argument("--offline", action="store_true", help="Resolve Cargo metadata using cached crates only")
    args = parser.parse_args()
    generator = Generator(check=args.check, offline=args.offline)
    try:
        inventory = generator.run()
        stale = generator.write_or_check()
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"Notice generation failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError):
            print(error.stderr.decode("utf-8", errors="replace"), file=sys.stderr)
        return 2
    print(json.dumps(inventory["counts"], sort_keys=True))
    for issue in inventory["validation"]["errors"]:
        print(f"ERROR: {issue}", file=sys.stderr)
    print(f"Review issues: {len(inventory['validation']['reviewIssues'])}")
    if args.check and stale:
        print("Stale generated files: " + ", ".join(stale), file=sys.stderr)
    return int(bool(inventory["validation"]["errors"] or (args.check and stale) or
                    (args.strict and inventory["validation"]["reviewIssues"])))


if __name__ == "__main__":
    raise SystemExit(main())
