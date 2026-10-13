"""Focused coverage and reproducibility tests; no network, builds, or installs."""
import base64
import copy
import contextlib
import io
import json
from pathlib import Path
import tempfile
import tarfile
import unittest
from unittest import mock

from generate_notices import Generator, cargo_closures, json_bytes, license_files, main, notice_comments, production_npm, safe_relative, sha, verify_published_crate


def graph(edges, macros=()):
    names = sorted(set(edges) | {name for deps in edges.values() for name, _ in deps})
    return {
        "packages": [{"id": name, "targets": [{"kind": ["proc-macro" if name in macros else "lib"]}]} for name in names],
        "resolve": {"root": "app", "nodes": [
            {"id": name, "deps": [{"pkg": dep, "dep_kinds": [{"kind": kind, "target": None}]} for dep, kind in edges.get(name, [])]}
            for name in names]},
    }


class InputHashTests(unittest.TestCase):
    def test_source_inputs_hash_the_same_on_crlf_and_lf_checkouts(self):
        # Git's text=auto checks source out with CRLF on Windows; the gate must not churn.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "licenses").mkdir()
            files = {"lf.md": b"a\nb\n", "crlf.md": b"a\r\nb\r\n", "image.bin": b"\x00\r\n",
                     "licenses/upstream.txt": b"a\r\nb\r\n"}
            generator = Generator(root=root)
            for name, data in files.items():
                (root / name).write_bytes(data)
                generator.input(root / name)
            self.assertEqual(generator.inputs["crlf.md"], generator.inputs["lf.md"])
            # Binary files and byte-exact license evidence keep their real digest.
            self.assertEqual(generator.inputs["image.bin"], sha(files["image.bin"]))
            self.assertEqual(generator.inputs["licenses/upstream.txt"], sha(files["licenses/upstream.txt"]))


class ClosureTests(unittest.TestCase):
    def test_excludes_build_dev_and_proc_macro_helpers(self):
        metadata = graph({"app": [("runtime", None), ("builder", "build"), ("test", "dev"), ("derive", None)],
                          "runtime": [("shared", None)], "builder": [("shared", None), ("cc", None)],
                          "derive": [("syn", None)]}, macros={"derive"})
        normal, tools = cargo_closures(metadata)
        self.assertEqual(normal, {"runtime", "shared"})
        self.assertEqual(tools, {"builder", "cc", "derive", "syn"})
        self.assertNotIn("test", normal | tools)

    def test_normal_use_wins_over_build_use_and_cycles_terminate(self):
        metadata = graph({"app": [("shared", "build"), ("a", None)], "a": [("shared", None)], "shared": [("a", None)]})
        self.assertEqual(cargo_closures(metadata), ({"a", "shared"}, set()))

    def test_dependencies_of_build_dependencies_stay_build_only(self):
        metadata = graph({"app": [("builder", "build")], "builder": [("native_tool", None)]})
        self.assertEqual(cargo_closures(metadata), (set(), {"builder", "native_tool"}))

    def test_platform_graphs_are_walked_independently(self):
        windows = graph({"app": [("windows", None)]})
        mac = graph({"app": [("metal", None)]})
        self.assertEqual(cargo_closures(windows)[0], {"windows"})
        self.assertEqual(cargo_closures(mac)[0], {"metal"})

    def test_npm_excludes_dev_only_but_keeps_optional_production(self):
        lock = {"lockfileVersion": 3, "packages": {"": {}, "node_modules/prod": {}, "node_modules/dev": {"dev": True},
                "node_modules/optional": {"optional": True}, "node_modules/shared-optional": {"devOptional": True},
                "node_modules/dev-optional": {"dev": True, "optional": True}}}
        self.assertEqual([p for p, _ in production_npm(lock)], ["node_modules/optional", "node_modules/prod", "node_modules/shared-optional"])

    def test_old_lock_format_fails_instead_of_empty_success(self):
        with self.assertRaises(ValueError):
            production_npm({"lockfileVersion": 1, "dependencies": {"shipping": {}}})


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.generator = Generator(self.root)

    def test_verbatim_bytes_are_deduplicated_by_content_not_license_name(self):
        data = b"Copyright A  \r\nPermission ...\r\n"
        a = self.generator.text(data, "https://one/LICENSE", "LICENSE")
        b = self.generator.text(data, "https://two/LICENSE", "LICENSE")
        c = self.generator.text(b"Copyright B\nPermission ...\n", "https://three/LICENSE", "LICENSE")
        self.assertEqual(a["sha256"], b["sha256"])
        self.assertNotEqual(a["sha256"], c["sha256"])
        self.assertEqual(self.generator.outputs[a["path"]], data)
        self.assertEqual(len(self.generator.texts), 2)

    def test_empty_text_cannot_satisfy_notice_coverage(self):
        self.assertEqual(self.generator.text(b" \r\n", "source", "LICENSE"), {})
        self.assertIn("Empty license/notice text: source", self.generator.errors)

    def test_markdown_fences_cannot_be_closed_by_license_text(self):
        data = b"Copyright A\r\n```\r\nLicense text without final newline"
        record = self.generator.text(data, "source", "LICENSE")
        self.generator.components = [{"id": "rust:a@1", "kind": "rust", "name": "a", "license": "MIT", "texts": [record]}]
        output = self.generator.render({"downloads": [], "validation": {"errors": []}, "services": [{"name": "DO_NOT_PUBLISH_SERVICE"}]})
        self.assertIn(b"````text\n" + data + b"\n````", output)
        self.assertNotIn(b"DO_NOT_PUBLISH_SERVICE", output)

    def test_explicit_legacy_encoding_keeps_original_and_renders_characters(self):
        data = b"Copyright \xa9 2000\r\n"
        record = self.generator.text(data, "source", "LICENSE", encoding="windows-1252")
        self.generator.components = [{"id": "installer:one", "kind": "installer", "name": "One", "license": "Zlib", "texts": [record]}]
        output = self.generator.render({"downloads": [], "validation": {"errors": []}})
        self.assertEqual(self.generator.outputs[record["path"]], data)
        self.assertIn("Copyright © 2000\r\n".encode(), output)

    def test_native_notice_blocks_keep_copyright_and_full_terms(self):
        data = b"// Copyright Example\r\n// permission terms\r\nint a;\n/* Copyright Other\n * All rights reserved.\n */\n"
        comments = notice_comments(data)
        self.assertEqual(len(comments), 2)
        for start, end, comment in comments:
            self.assertEqual(data[start:end], comment)
        self.assertTrue(comments[0][2].endswith(b"terms\r\n"))

    def test_nested_notices_are_found_without_test_or_node_modules_noise(self):
        for relative in ["LICENSE", "vendor/NOTICE", "vendor/LICENSE-MIT", "tests/LICENSE", "node_modules/noise/LICENSE"]:
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("terms")
        self.assertEqual({p.relative_to(self.root).as_posix() for p in license_files(self.root)}, {"LICENSE", "vendor/NOTICE", "vendor/LICENSE-MIT"})

    def test_check_reports_stale_output_without_writing(self):
        self.generator.check = True
        self.generator.emit("licenses/inventory.json", b"new\n")
        path = self.root / "licenses/inventory.json"
        path.parent.mkdir()
        path.write_bytes(b"old\n")
        self.assertEqual(self.generator.write_or_check(), ["licenses/inventory.json"])
        self.assertEqual(path.read_bytes(), b"old\n")

    def test_retired_generated_text_is_stale_then_removed_on_regeneration(self):
        path = self.root / "licenses/texts/old.txt"
        path.parent.mkdir(parents=True)
        path.write_bytes(b"retired dependency")
        self.generator.check = True
        self.assertEqual(self.generator.write_or_check(), ["licenses/texts/old.txt"])
        self.assertTrue(path.exists())
        self.generator.check = False
        self.generator.write_or_check()
        self.assertFalse(path.exists())

    def test_upstream_evidence_hash_tampering_fails(self):
        path = self.root / "licenses/upstream-evidence.json"
        path.parent.mkdir()
        path.write_bytes(json_bytes({"records": [{"ids": [], "url": "https://upstream/LICENSE", "sha256": sha(b"original"),
                                                  "base64": base64.b64encode(b"altered").decode()}]}))
        with self.assertRaisesRegex(ValueError, "hash mismatch"):
            self.generator.upstream_evidence()

    def test_spdx_declaration_does_not_pass_as_full_license(self):
        text = self.generator.text(b"SPDXVersion: SPDX-2.1\nPackageLicenseDeclared: MIT\n", "source", "LICENSE.spdx")
        self.generator.components = [{"id": "npm:a@1", "kind": "npm", "name": "a", "license": "MIT", "texts": [text]}]
        self.generator.validate()
        self.assertIn("Missing full license text: npm:a@1", self.generator.errors)

    def test_new_native_crate_requires_explicit_coverage(self):
        directory = self.root / "crate"
        directory.mkdir()
        (directory / "native.c").write_text("int native(void) { return 1; }")
        (self.root / "licenses").mkdir()
        (self.root / "licenses/native-rules.json").write_bytes(json_bytes({"crates": {}}))
        self.generator.components = [{"id": "rust:crate@1", "name": "crate", "version": "1", "kind": "rust", "features": {}, "texts": []}]
        self.generator.packages = {"rust:crate@1": {"manifest_path": str(directory / "Cargo.toml")}}
        self.generator.native_audit()
        self.assertIn("Native source/binary has no reviewed coverage rule: rust:crate@1", self.generator.errors)

    def test_npm_same_version_at_two_lock_locations_is_one_component(self):
        locations = ["node_modules/a", "node_modules/outer/node_modules/a"]
        lock = {"lockfileVersion": 3, "packages": {location: {"version": "1", "license": "MIT", "integrity": "same"} for location in locations}}
        (self.root / "package-lock.json").write_bytes(json_bytes(lock))
        (self.root / "package.json").write_bytes(b"{}")
        for location in locations:
            directory = self.root / location
            directory.mkdir(parents=True)
            (directory / "package.json").write_bytes(json_bytes({"name": "a", "version": "1"}))
            (directory / "LICENSE").write_text("MIT terms")
        self.generator.npm()
        self.assertEqual(len(self.generator.components), 1)
        self.assertEqual(self.generator.components[0]["lockPaths"], locations)
        self.assertFalse(self.generator.errors)


class DownloadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "licenses/downloads").mkdir(parents=True)
        self.generator = Generator(self.root)

    def write_input(self, entry):
        path = self.root / "licenses/download-components.json"
        path.write_bytes(json_bytes({"schemaVersion": 1, "customTopField": "kept", "components": [entry], "services": [{"name": "host"}]}))

    def test_merge_preserves_supplier_fields_and_original_text(self):
        data = b"Copyright Provider\r\nLicense\r\n"
        (self.root / "licenses/downloads/license.txt").write_bytes(data)
        entry = {"id": "model", "kind": "model", "name": "Model", "license": "MIT", "licenseUrl": "https://upstream/license",
                 "textFiles": ["downloads/license.txt"], "catalogIds": ["model"], "custom": {"proof": [1, 2]}}
        expected = copy.deepcopy(entry)
        self.write_input(entry)
        components, services, metadata = self.generator.downloads()
        for key, value in expected.items():
            self.assertEqual(components[0][key], value)
        self.assertEqual(components[0]["noticeTexts"][0]["sha256"], sha(data))
        self.assertEqual(services, [{"name": "host"}])
        self.assertEqual(metadata["customTopField"], "kept")
        self.assertFalse(self.generator.errors)

    def test_missing_input_and_empty_texts_are_errors(self):
        self.generator.downloads()
        self.assertTrue(any("Missing mandatory" in e for e in self.generator.errors))
        self.generator.errors.clear()
        self.write_input({"id": "model", "kind": "model", "license": "Unverified", "textFiles": []})
        self.generator.downloads()
        self.assertIn("Unverified download license: model", self.generator.errors)
        self.assertIn("No license/notice text supplied for download: model", self.generator.errors)

    def evidence(self, saved, original=None):
        (self.root / "licenses/downloads/license.txt").write_bytes(saved)
        record = {"path": "downloads/license.txt", "url": "https://exact-source/NOTICE",
                  "savedSha256": sha(saved), "savedBytes": len(saved),
                  "sourceSha256": sha(original or saved), "sourceBytes": len(original or saved)}
        if original is not None:
            record["originalBytesBase64"] = base64.b64encode(original).decode()
        (self.root / "licenses/downloads/evidence.json").write_bytes(json_bytes({"files": [record]}))

    def test_download_evidence_recovers_original_bytes_and_exact_source(self):
        original = b"Copyright Owner\r\nPermission ...\r\n"
        self.evidence(original.replace(b"\r\n", b"\n"), original)
        self.assertEqual(self.generator.download_evidence("downloads/evidence.json")["downloads/license.txt"],
                         (original, "https://exact-source/NOTICE"))

    def test_download_notice_uses_each_files_source_and_original_bytes(self):
        original = b"Copyright Owner\r\nPermission ...\r\n"
        self.evidence(original.replace(b"\r\n", b"\n"), original)
        self.write_input({"id": "model", "kind": "model", "license": "MIT", "licenseUrl": "https://generic/license",
                          "textFiles": ["downloads/license.txt"]})
        path = self.root / "licenses/download-components.json"
        data = json.loads(path.read_bytes())
        data["evidenceManifest"] = "downloads/evidence.json"
        path.write_bytes(json_bytes(data))
        components, _, _ = self.generator.downloads()
        notice = components[0]["noticeTexts"][0]
        self.assertEqual(notice["source"], "https://exact-source/NOTICE")
        self.assertEqual(notice["sha256"], sha(original))
        self.assertFalse(self.generator.errors)

    def test_download_evidence_saved_file_tampering_fails(self):
        self.evidence(b"Original terms\n")
        (self.root / "licenses/downloads/license.txt").write_bytes(b"Altered terms\n")
        with self.assertRaisesRegex(ValueError, "saved hash/size mismatch"):
            self.generator.download_evidence("downloads/evidence.json")

    def test_download_evidence_cannot_rewrite_legal_wording(self):
        self.evidence(b"Different terms\n", b"Original terms\n")
        with self.assertRaisesRegex(ValueError, "differs beyond newline normalization"):
            self.generator.download_evidence("downloads/evidence.json")

    def test_evidence_paths_cannot_escape_license_directory(self):
        for value in ("../secret", "/tmp/secret", "C:/secret", "C:\\secret", "downloads/../../secret", ""):
            with self.subTest(value=value), self.assertRaises(ValueError):
                safe_relative(value)
        self.assertEqual(safe_relative("downloads/LICENSE"), Path("downloads/LICENSE"))


class ExitStatusTests(unittest.TestCase):
    def status(self, flags=(), errors=(), reviews=(), stale=()):
        inventory = {"counts": {}, "validation": {"errors": list(errors), "reviewIssues": list(reviews)}}
        with mock.patch("sys.argv", ["generate_notices.py", *flags]), mock.patch.object(Generator, "run", return_value=inventory), \
                mock.patch.object(Generator, "write_or_check", return_value=list(stale)), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return main()

    def test_mandatory_coverage_failure_is_nonzero_even_during_generation(self):
        self.assertEqual(self.status(errors=["Missing license"]), 1)

    def test_check_fails_on_stale_outputs(self):
        self.assertEqual(self.status(flags=["--check"], stale=["licenses/inventory.json"]), 1)

    def test_strict_gate_fails_on_unresolved_review(self):
        self.assertEqual(self.status(flags=["--strict", "--check"], reviews=["Unverified ownership"]), 1)

    def test_clean_inventory_passes(self):
        self.assertEqual(self.status(flags=["--check"]), 0)


class MplSourceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "src").mkdir()
        (self.root / "src/lib.rs").write_bytes(b"// MPL-2.0\npub fn answer() {}\n")
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode="w:gz") as archive:
            data = (self.root / "src/lib.rs").read_bytes()
            entry = tarfile.TarInfo("example-1.0/src/lib.rs")
            entry.size = len(data)
            archive.addfile(entry, io.BytesIO(data))
        self.archive = stream.getvalue()

    def verify(self):
        return verify_published_crate(self.root, self.archive, sha(self.archive), "example-1.0")

    def test_unmodified_source_and_cargo_metadata_pass(self):
        (self.root / ".cargo-ok").write_text("{}")
        self.assertEqual(self.verify()["verifiedFileCount"], 1)
        self.assertTrue(self.verify()["unmodified"])

    def test_modified_source_fails_even_when_archive_matches_lock(self):
        (self.root / "src/lib.rs").write_text("pub fn altered() {}")
        with self.assertRaisesRegex(ValueError, "Modified or missing"):
            self.verify()

    def test_added_source_fails(self):
        (self.root / "build.rs").write_text("fn main() {}")
        with self.assertRaisesRegex(ValueError, "Extra files"):
            self.verify()

    def test_archive_must_match_lock(self):
        with self.assertRaisesRegex(ValueError, "checksum differs"):
            verify_published_crate(self.root, self.archive, "0" * 64, "example-1.0")

    def test_notices_link_versioned_sources_and_mark_unmodified(self):
        generator = Generator(self.root)
        generator.components = [{"id": "rust:example@1.0", "kind": "rust", "name": "example", "version": "1.0",
            "license": "MPL-2.0", "texts": [], "sourceAvailability": {"unmodified": True, "verifiedFileCount": 1,
            "archiveSha256": sha(self.archive), "versionUrl": "https://crates.io/crates/example/1.0",
            "sourceUrl": "https://crates.io/api/v1/crates/example/1.0/download"}}]
        result = generator.render({"validation": {"errors": []}, "downloads": []}).decode()
        self.assertIn("## Source for MPL-2.0 components", result)
        self.assertIn("https://crates.io/api/v1/crates/example/1.0/download", result)
        self.assertIn("unmodified, 1 files verified", result)


class ComplianceNoticeTests(unittest.TestCase):
    def test_punctuation_and_ffmpeg_evidence_reaches_offline_notices(self):
        # Real captured manifest: this catches accidentally dropping a notice from textFiles.
        from generate_notices import ROOT
        generator = Generator(ROOT)
        downloads, _, _ = generator.downloads()
        wanted = {"sonojot-punct-v1", "sonojot-punct-v2", "ffmpeg-windows", "ffmpeg-macos"}
        selected = [c for c in downloads if c["id"] in wanted]
        self.assertEqual(len(selected), 4)
        for component in selected:
            result = generator.render({"validation": {"errors": []}, "downloads": [component]}).decode()
            if component["id"].startswith("sonojot-punct"):
                for text in ("SonoJot", "Rev.com", "Earnings-21", "Horizon-Labs", "mmBERT",
                             "https://creativecommons.org/licenses/by-sa/4.0/legalcode.en", "Modified models must remain CC BY-SA"):
                    self.assertIn(text, result)
                if component["id"].endswith("v2"):
                    self.assertIn("Earnings-22", result)
            else:
                self.assertIn("GNU", result)
                self.assertIn("FFmpeg", result)


if __name__ == "__main__":
    unittest.main()
