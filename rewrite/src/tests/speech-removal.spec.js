// October 14 owner feedback from the Mac: the only speech model's Remove says
// plainly why it waits and offers the fix; another installed speech model
// unblocks it; a repair download names the shared add-on it is fetching.
import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { openApp, openModelLibrary } from "./app-helpers.js";
import { macProfileFixture } from "./desktop-fixture.js";

const q4 = "qwen3-asr-1.7b-q4-metal";
const whisper = "whisper-small-metal";
const blocked =
  "SonoJot needs a speech model for dictation. To remove this one, install another speech model first.";

async function screenshot(page, name, locator = page) {
  const directory = new URL("../../../artifacts/scroll-review/", import.meta.url);
  await mkdir(directory, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await locator.screenshot({
    path: fileURLToPath(new URL(`${name}.png`, directory)),
  });
}

/** A Mac with Qwen Q4 installed and selected; Whisper as given. */
function mac({ whisperState = "available", qwen = {} } = {}) {
  const fixture = macProfileFixture(16);
  return {
    ...fixture,
    settings: { ...fixture.settings, setupProfile: "custom", asrModel: q4 },
    libraryModels: fixture.libraryModels.map((model) =>
      model.id === q4
        ? { ...model, state: "installed", removable: true, ...qwen }
        : model.id === whisper
          ? { ...model, state: whisperState, removable: whisperState === "installed" }
          : model,
    ),
    modelPatches: {
      ...fixture.modelPatches,
      // Files a Mac cannot run: still on disk, never a replacement.
      "moonshine-medium": {
        supported: false,
        installable: false,
        compatibility:
          "This package requires Windows; choose a Metal model on this Mac.",
      },
      ...Object.fromEntries(
        [
          "mock-whisper-small-cpu",
          "mock-whisper-small-cuda",
          "qwen3-asr-1.7b",
          "qwen3-asr-1.7b-fp16-cuda",
          "whisper-tiny-fp32-cuda",
        ].map((id) => [
          id,
          {
            state: "unavailable",
            installable: false,
            compatibility:
              "This package requires Windows; choose a Metal model on this Mac.",
          },
        ]),
      ),
    },
  };
}

const settings = (page) =>
  page.evaluate(() => window.__testSnapshot().settings);
const calls = (page, command) =>
  page.evaluate(
    (name) => window.__testCalls.filter((call) => call.command === name),
    command,
  );

test("the only speech model explains its disabled Remove and shows speech models", async ({
  page,
}) => {
  await openApp(page, mac(), "/?page=setup&theme=fog");
  const library = await openModelLibrary(page);
  await library
    .getByRole("searchbox", { name: "Search model library" })
    .fill("Q4 Metal");
  const row = library.locator(`[data-model-id="${q4}"]`);
  await expect(
    row.getByRole("button", { name: /^Remove Qwen3-ASR/ }),
  ).toBeDisabled();
  await expect(row.getByRole("status")).toHaveText(blocked);
  await screenshot(page, "after-remove-only-speech-row", row);
  await row.getByRole("button", { name: "Show speech models", exact: true }).click();
  // Get more, filtered to speech, with the installable Metal models listed.
  await expect(
    library.getByRole("tab", { name: "Get more", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(
    library.getByRole("combobox", { name: "Filter model library" }),
  ).toHaveValue("asr");
  await expect(
    library.getByRole("searchbox", { name: "Search model library" }),
  ).toHaveValue("");
  const offer = library.locator(`[data-model-id="${whisper}"]`);
  await expect(offer.getByRole("button", { name: /^Install / })).toBeEnabled();
  expect(await calls(page, "remove_model")).toHaveLength(0);
});

test("another installed speech model enables Remove and takes over Speech", async ({
  page,
}) => {
  await openApp(page, mac({ whisperState: "installed" }), "/?page=setup&theme=fog");
  const library = await openModelLibrary(page);
  await library
    .getByRole("searchbox", { name: "Search model library" })
    .fill("Q4 Metal");
  const row = library.locator(`[data-model-id="${q4}"]`);
  await expect(row.getByRole("status")).toHaveCount(0);
  const remove = row.getByRole("button", { name: /^Remove Qwen3-ASR/ });
  await expect(remove).toBeEnabled();
  await remove.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText(
    "Speech will switch to Whisper Small · Metal. Its downloaded model files will be deleted.",
  );
  await dialog.getByRole("button", { name: "Remove", exact: true }).click();
  await expect
    .poll(async () => (await calls(page, "remove_model")).length)
    .toBe(1);
  expect((await settings(page)).asrModel).toBe(whisper);
});

test("a repair download names the speech detector, then the model files", async ({
  page,
}) => {
  await openApp(
    page,
    mac({
      qwen: {
        state: "downloading",
        removable: false,
        downloadedBytes: 29e6,
        totalBytes: 33e6,
        stage: "speech detector",
      },
    }),
    "/?page=setup&theme=fog",
  );
  const download = page.locator(`section.download[data-model-id="${q4}"]`).first();
  await expect(download.getByRole("status")).toHaveText(
    "Downloading speech detector · 87%",
  );
  await expect(download).toContainText("29 MB / 33 MB");
  await screenshot(page, "after-download-detector", download);
  await page.evaluate(
    (id) =>
      window.__testPatchModel(id, {
        stage: "model files",
        downloadedBytes: 800e6,
        totalBytes: 1700e6,
      }),
    q4,
  );
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(download.getByRole("status")).toHaveText(
    "Downloading model files · 47%",
  );
});
