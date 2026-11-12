import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture, macProfileFixture } from "./desktop-fixture.js";
import { qwen, parakeet, cpu, model, pc } from "./speech-fixture.js";

// SonoJot Parakeet (ft2) as the main choice against Qwen3-ASR once its weights
// can be downloaded. Awaiting director review; screenshots for that review.
const folder = new URL("../../artifacts/parakeet-ft-desktop/", import.meta.url);
const ft2 = "parakeet-tdt-0.6b-v2-sonojot-ft2";
const sonojot = (patch = {}) => ({
  ...model(ft2, "SonoJot Parakeet", 2.1e9),
  family: "Parakeet",
  languages: ["en"],
  diskBytes: 661_190_591,
  backend: "sherpa-onnx-cpu",
  quantization: "int8",
  state: "available",
  downloadedBytes: 0,
  totalBytes: 661_190_591,
  removable: false,
  ...patch,
});

async function open(page, fixture, theme = "fog") {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await installDesktopFixture(page, fixture);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(`/?page=setup&theme=${theme}`);
  await expect(
    page.getByRole("heading", { name: "Speech", exact: true }),
  ).toBeVisible();
  expect(errors).toEqual([]);
}
async function shot(page, name) {
  await mkdir(folder, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(name, folder)) });
}
const card = (page, id) => page.locator(`[data-model-id="${id}"]`);

/** The native frontier when SonoJot Parakeet is offered and the default switch is OFF. */
function offered(fixture, dictation) {
  return {
    ...fixture,
    libraryModels: [...fixture.libraryModels, sonojot()],
    hardwareProfile: {
      ...fixture.hardwareProfile,
      frontier: { ...fixture.hardwareProfile.frontier, dictation },
    },
  };
}

test("NVIDIA: SonoJot Parakeet against Qwen, Qwen still recommended", async ({
  page,
}) => {
  await open(page, offered(pc(), [ft2, qwen]));
  await expect(card(page, ft2)).toContainText("SonoJot Parakeet");
  await expect(card(page, ft2)).toContainText("tuned by SonoJot for dictation");
  await expect(card(page, qwen)).toContainText("Recommended");
  await expect(card(page, parakeet)).toHaveCount(0);
  await shot(page, "01-nvidia-main-choice.png");
  await page.getByRole("button", { name: "All models", exact: true }).click();
  await expect(card(page, parakeet)).toBeVisible();
  await shot(page, "02-nvidia-all-models.png");
});

test("CPU only: SonoJot Parakeet against Qwen, stock Parakeet kept while recommended", async ({
  page,
}) => {
  await open(page, offered(pc(false), [ft2, cpu, parakeet]));
  await expect(card(page, ft2)).toBeVisible();
  await expect(card(page, parakeet)).toContainText("Recommended");
  await shot(page, "03-cpu-main-choice.png");
});

test("Mac: SonoJot Parakeet against Qwen Q4 and Q8", async ({ page }) => {
  const mac = macProfileFixture(16);
  const fixture = offered(
    {
      ...mac,
      firstRun: false,
      settings: {
        ...mac.settings,
        setupProfile: "balanced",
        asrModel: "qwen3-asr-1.7b-q4-metal",
        localProofreader: false,
      },
      modelPatches: {
        ...mac.modelPatches,
        "qwen3-asr-1.7b-q4-metal": { state: "installed" },
      },
    },
    [ft2, "qwen3-asr-1.7b-q4-metal", "qwen3-asr-1.7b-q8-metal"],
  );
  await open(page, fixture);
  await expect(card(page, ft2)).toBeVisible();
  await expect(card(page, "qwen3-asr-1.7b-q4-metal")).toContainText(
    "Recommended",
  );
  await shot(page, "04-mac-main-choice.png");
});

test("Before the weights are published: listed under All models as not published", async ({
  page,
}) => {
  const fixture = pc();
  await open(page, {
    ...fixture,
    libraryModels: [
      ...fixture.libraryModels,
      sonojot({
        installable: false,
        error:
          "Not published yet. SonoJot has not released these weights for download.",
      }),
    ],
  });
  await expect(card(page, ft2)).toHaveCount(0);
  await page.getByRole("button", { name: "All models", exact: true }).click();
  await expect(card(page, ft2)).toContainText("Not published yet");
  await expect(card(page, ft2)).toBeDisabled();
  await card(page, ft2).scrollIntoViewIfNeeded();
  await shot(page, "05-unpublished-all-models.png");
});
