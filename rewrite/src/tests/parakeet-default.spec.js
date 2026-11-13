import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture, macProfileFixture } from "./desktop-fixture.js";
import { qwen, cpu, model, pc } from "./speech-fixture.js";

// November 12, 2026 owner decision: on English systems Balanced (the default
// profile) recommends SonoJot Parakeet on every computer, NVIDIA and Apple
// Silicon included; meetings follow ("Same as dictation"). Qwen3-ASR stays one
// tap away as the multilingual choice. Awaiting director review; these are the
// screenshots for it (artifacts/ is git-ignored).
const folder = new URL("../../../artifacts/parakeet-default/", import.meta.url);
test.setTimeout(90_000);
const ft2 = "parakeet-tdt-0.6b-v2-sonojot-ft2";
const q4 = "qwen3-asr-1.7b-q4-metal";
const q8 = "qwen3-asr-1.7b-q8-metal";
const FT2_REASON =
  "SonoJot Parakeet is tuned by SonoJot for English dictation and meetings. It is smaller and faster than Qwen and finishes moments after you stop speaking.";
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

/** The native profile once SonoJot Parakeet is the English default. */
function english(fixture, { dictation, other, firstRun }) {
  const recommendations = fixture.hardwareProfile.recommendations;
  const balanced = {
    ...recommendations.balanced,
    asrModel: ft2,
    reasons: [FT2_REASON],
  };
  return {
    ...fixture,
    firstRun,
    settings: firstRun
      ? { ...fixture.settings, setupProfile: null }
      : { ...fixture.settings, setupProfile: "balanced", asrModel: ft2 },
    libraryModels: [
      ...fixture.libraryModels,
      sonojot(
        firstRun ? {} : { state: "installed", downloadedBytes: 661_190_591 },
      ),
    ],
    hardwareProfile: {
      ...fixture.hardwareProfile,
      recommendations: { ...recommendations, balanced },
      frontier: {
        dictation,
        meetings: fixture.hardwareProfile.frontier.meetings.includes(
          "granite-speech-4.1-2b-nar-q8",
        )
          ? [...dictation, "granite-speech-4.1-2b-nar-q8"]
          : dictation,
        otherLanguages: other,
      },
    },
  };
}

async function shot(page, name) {
  await mkdir(folder, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(name, folder)) });
}
// The choose button of a speech card; its information button sits beside it.
const card = (page, id) =>
  page.locator(`[data-model-id="${id}"] .card-choose`);

async function firstRunModels(page, fixture, prefix) {
  await page.clock.install();
  await installDesktopFixture(page, fixture);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/?page=setup&theme=fog", { waitUntil: "domcontentloaded" });
  const go = async (name = "Continue") => {
    await page.clock.runFor(5000);
    await page.getByRole("button", { name, exact: true }).click();
  };
  await go();
  await page
    .getByRole("checkbox", { name: "I agree to the SonoJot Terms of Use" })
    .check();
  await page
    .getByRole("button", { name: "Agree and continue", exact: true })
    .click();
  // Microphone, shortcut, dictation (and Mac permissions) until Models.
  const balancedButton = page.getByRole("button", {
    name: "Balanced Recommended",
  });
  for (let step = 0; step < 6 && !(await balancedButton.count()); step++)
    await go();
  await expect(balancedButton).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByText(
      /Balanced uses SonoJot Parakeet, tuned by SonoJot for English dictation and meetings/,
    ),
  ).toBeVisible();
  await expect(
    page.getByText(
      /faster and smaller than Qwen3-ASR, the multilingual choice/,
    ),
  ).toBeVisible();
  await shot(page, `${prefix}-1-setup-models.png`);
  await go("Use Balanced");
  await expect(
    page.getByRole("heading", { name: "Downloads", exact: true, level: 1 }),
  ).toBeVisible();
  const speech = page
    .getByRole("list", { name: "Setup downloads" })
    .getByRole("listitem")
    .filter({ hasText: "SonoJot Parakeet" });
  await expect(speech).toHaveCount(1);
  await expect(speech).toContainText(/66\d MB/);
  await shot(page, `${prefix}-2-setup-downloads.png`);
}

async function speechScreen(page, fixture, prefix, qwenId) {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await installDesktopFixture(page, fixture);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/?page=setup&theme=fog");
  await expect(
    page.getByRole("heading", { name: "Speech", exact: true }),
  ).toBeVisible();
  expect(errors).toEqual([]);
  await expect(card(page, ft2)).toContainText("Recommended");
  await expect(card(page, ft2)).toContainText("English only. Fast and small.");
  // The longer claim lives behind the information button.
  await page.getByRole("button", { name: "About SonoJot Parakeet" }).click();
  await expect(
    page.getByRole("note", { name: "About SonoJot Parakeet" }),
  ).toContainText(
    "tuned by SonoJot for English dictation and meetings. Faster and smaller than Qwen3-ASR",
  );
  await page.keyboard.press("Escape");
  await expect(card(page, qwenId)).toContainText("Many languages");
  await expect(card(page, qwenId)).not.toContainText("Recommended");
  await shot(page, `${prefix}-3-speech.png`);
}

test("NVIDIA PC in English: SonoJot Parakeet is the default, Qwen GPU the multilingual choice", async ({
  page,
}) => {
  const options = { dictation: [ft2, qwen], other: qwen };
  await firstRunModels(
    page,
    english(pc(), { ...options, firstRun: true }),
    "nvidia",
  );
});
test("NVIDIA PC in English: Speech picker", async ({ page }) => {
  const fixture = english(pc(), {
    dictation: [ft2, qwen],
    other: qwen,
    firstRun: false,
  });
  await speechScreen(page, fixture, "nvidia", qwen);
  await expect(card(page, qwen)).toContainText(
    "fewer errors on prepared talks",
  );
});

test("CPU-only PC in English: SonoJot Parakeet is the default", async ({
  page,
}) => {
  await firstRunModels(
    page,
    english(pc(false), { dictation: [ft2, cpu], other: cpu, firstRun: true }),
    "cpu",
  );
});
test("CPU-only PC in English: Speech picker", async ({ page }) => {
  await speechScreen(
    page,
    english(pc(false), { dictation: [ft2, cpu], other: cpu, firstRun: false }),
    "cpu",
    cpu,
  );
});

test("Apple Silicon Mac in English: SonoJot Parakeet is the default", async ({
  page,
}) => {
  const mac = macProfileFixture(16);
  await firstRunModels(
    page,
    english(mac, { dictation: [ft2, q4, q8], other: q4, firstRun: true }),
    "mac",
  );
});
test("Apple Silicon Mac in English: Speech picker", async ({ page }) => {
  const mac = macProfileFixture(16);
  const fixture = english(
    { ...mac, settings: { ...mac.settings, localProofreader: false } },
    { dictation: [ft2, q4, q8], other: q4, firstRun: false },
  );
  await speechScreen(page, fixture, "mac", q4);
  // Measured on an M4 Mac mini on November 13, 2026: Compare models plots
  // SonoJot Parakeet, marked recommended, with its Mac numbers.
  await page.getByRole("button", { name: "Compare models" }).click();
  const point = page.locator(`[data-chart-model="${ft2}"]`);
  await point.focus();
  await expect(page.locator(".readout")).toContainText("Recommended");
  await expect(page.locator(".readout")).toContainText(
    "3.73 word errors per 100 words",
  );
  await expect(page.locator(".readout")).toContainText("waits 0.23 s");
});
