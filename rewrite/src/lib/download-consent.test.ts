import assert from "node:assert/strict";
import test from "node:test";
import {
  downloadConsentGroups,
  pendingDownloadConsents,
} from "./download-consent";

test("old settings and changed pins need acknowledgment; retries with the same pin do not", () => {
  const ids = ["granite-speech-4.1-2b-nar-q8"];
  const [group] = pendingDownloadConsents(ids, "windows", {});
  assert.equal(group.id, "nvidia-granite-cuda12");
  assert.equal(
    pendingDownloadConsents(ids, "windows", { [group.id]: group.version })
      .length,
    0,
  );
  assert.equal(
    pendingDownloadConsents(ids, "windows", { [group.id]: "previous-version" })
      .length,
    1,
  );
  assert.equal(pendingDownloadConsents(ids, "macos", {}).length, 0);
});

test("both LFM sizes and every downloadable NVIDIA runtime family have explicit terms", () => {
  for (const id of [
    "lfm2.5-1.2b",
    "lfm2.5-2.6b",
    "ternary-bonsai-2-27b",
    "qwen3-asr-1.7b-q8-cuda",
    "qwen3-asr-1.7b-fp16-cuda",
    "whisper-tiny-fp32-cuda",
    "granite-speech-4.1-2b-nar-q8",
  ]) {
    assert.equal(pendingDownloadConsents([id], "windows", {}).length, 1, id);
  }
  assert.equal(
    pendingDownloadConsents(
      ["qwen3-asr-1.7b-q4-cpu", "moonshine-small", "sonojot-punct-v2"],
      "windows",
      {},
    ).length,
    0,
  );
});

test("groups deduplicate profile downloads and FFmpeg uses its platform pin", () => {
  assert.equal(
    pendingDownloadConsents(["lfm2.5-1.2b", "lfm2.5-1.2b"], "windows", {})
      .length,
    1,
  );
  assert.equal(
    pendingDownloadConsents(["ffmpeg"], "windows", {})[0].id,
    "ffmpeg-windows",
  );
  assert.equal(
    pendingDownloadConsents(["ffmpeg"], "macos", {})[0].id,
    "ffmpeg-macos",
  );
  assert.equal(
    new Set(downloadConsentGroups.map((group) => group.id)).size,
    downloadConsentGroups.length,
  );
  for (const group of downloadConsentGroups) {
    assert.match(group.version, /^[a-f0-9]{64}$/);
    assert.ok(
      group.links.every((link) => new URL(link.url).protocol === "https:"),
    );
  }
});
