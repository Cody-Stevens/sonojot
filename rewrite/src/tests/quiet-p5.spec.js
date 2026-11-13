import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture, macProfileFixture } from "./desktop-fixture.js";

const folder = new URL("../../artifacts/redesign-p5/", import.meta.url);
import { qwen, parakeet, cpu, granite, pc } from "./speech-fixture.js";
async function open(page, fixture, meeting = false, theme = "fog") {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await installDesktopFixture(page, fixture);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(`/?page=${meeting ? "meetings" : "setup"}&theme=${theme}`);
  if (meeting)
    await page
      .getByRole("button", { name: "Sources, speech and speakers" })
      .click();
  await expect(
    page.getByRole("heading", { name: "Speech", exact: true }),
  ).toBeVisible();
  // November 13: no chart on the page; one chart behind Compare models.
  await expect(
    page.getByRole("group", { name: "Speech models" }),
  ).toBeVisible();
  expect(errors).toEqual([]);
}
async function shot(page, name) {
  await mkdir(folder, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(name, folder)) });
}
// The choose button of a speech card; its information button sits beside it.
const card = (page, id) =>
  page.locator(`[data-model-id="${id}"] .card-choose`);
const settings = (page) =>
  page.evaluate(() => window.__testSnapshot().settings);
test("NVIDIA speech shows separate pools, measured waits and switching back", async ({
  page,
}) => {
  await open(page, pc());
  await expect(card(page, qwen)).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Compare models" }).click();
  await expect(
    page.getByRole("dialog", { name: "Compare models" }),
  ).toContainText("NVIDIA RTX 5070 · 12.0 GB graphics memory");
  await page.keyboard.press("Escape");
  await shot(page, "01-nvidia-speech.png");
  await card(page, parakeet).click();
  await expect.poll(async () => (await settings(page)).asrModel).toBe(parakeet);
  await page.getByRole("button", { name: /Back to Qwen3/ }).click();
  await expect.poll(async () => (await settings(page)).asrModel).toBe(qwen);
  await page.getByRole("button", { name: "All models", exact: true }).click();
  await card(page, cpu).click();
  await expect(page.getByRole("dialog")).toContainText("Not recommended here");
  await shot(page, "02-force-warning.png");
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  expect((await settings(page)).asrModel).toBe(qwen);
  await card(page, cpu).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Use anyway" })
    .click();
  await expect.poll(async () => (await settings(page)).asrModel).toBe(cpu);
});
test("CPU-only speech uses the CPU frontier without inventing a GPU", async ({
  page,
}) => {
  await open(page, pc(false));
  await expect(card(page, parakeet)).toContainText("Recommended");
  await page.getByRole("button", { name: "Compare models" }).click();
  const compare = page.getByRole("dialog", { name: "Compare models" });
  await expect(compare).toContainText("no supported graphics card");
  // No graphics marks without a graphics card.
  await expect(compare.locator(".mark.gpu")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await shot(page, "03-cpu-speech.png");
});
for (const ram of [16, 32])
  test(`Mac ${ram} GB keeps Q4 recommended and shared memory explicit`, async ({
    page,
  }) => {
    const fixture = macProfileFixture(ram);
    await open(
      page,
      {
        ...fixture,
        firstRun: false,
        settings: {
          ...fixture.settings,
          setupProfile: "balanced",
          asrModel: "qwen3-asr-1.7b-q4-metal",
          localProofreader: false,
        },
        modelPatches: {
          ...fixture.modelPatches,
          "qwen3-asr-1.7b-q4-metal": { state: "installed" },
        },
      },
      false,
      ram === 32 ? "slate" : "fog",
    );
    await expect(card(page, "qwen3-asr-1.7b-q4-metal")).toContainText(
      "Recommended",
    );
    await page.getByRole("button", { name: "Compare models" }).click();
    await expect(
      page.getByRole("dialog", { name: "Compare models" }),
    ).toContainText("Apple Silicon");
    await page.keyboard.press("Escape");
    await shot(page, `${ram === 16 ? "04" : "05"}-mac-${ram}-speech.png`);
  });
test("meeting speech retains source choice, speaker labels, AMI scores and shared online sheet", async ({
  page,
}) => {
  await open(page, pc(), true);
  await expect(
    page.getByRole("button", { name: /^Granite Speech 4.1/ }),
  ).toBeEnabled();
  await shot(page, "06-meeting-speech.png");
  await page.getByRole("button", { name: /^Granite Speech 4.1/ }).click();
  await expect
    .poll(async () => (await settings(page)).meetingAsrModel)
    .toBe(granite);
  await page.getByRole("button", { name: /^Same as dictation/ }).click();
  await expect
    .poll(async () => (await settings(page)).meetingAsrModel)
    .toBe("");
  await page.getByRole("button", { name: "You", exact: true }).click();
  await expect
    .poll(async () => (await settings(page)).meetingSource)
    .toBe("microphone");
  await page.getByRole("button", { name: /^Online · OpenRouter/ }).click();
  await expect(page.getByRole("dialog")).toContainText("OpenRouter");
});
test("whole-setup reset lists every override and retains connection and saved models", async ({
  page,
}) => {
  const fixture = pc();
  await open(page, {
    ...fixture,
    settings: {
      ...fixture.settings,
      setupProfile: "custom",
      proofreader: true,
      proofreaderModel: "provider/cleanup",
      meetingAsrModel: granite,
    },
    aiSettings: {
      backend: "openrouter",
      model: "provider/notes",
      speechModel: "provider/speech",
      connected: true,
      quickModels: [{ id: "provider/notes", role: "text" }],
    },
  });
  await page
    .getByRole("button", { name: "Back to recommended setup…" })
    .click();
  const sheet = page.getByRole("dialog");
  await expect(sheet).toContainText("Meeting speech");
  await expect(sheet).toContainText("Selected OpenRouter model");
  await expect(sheet).not.toContainText("provider/");
  await shot(page, "07-whole-setup-reset.png");
  await sheet
    .getByRole("button", { name: "Use recommended setup", exact: true })
    .click();
  const consent = page
    .getByRole("dialog")
    .filter({ has: page.getByRole("heading", { name: "Before downloading" }) });
  await consent.getByRole("checkbox").check();
  await consent.getByRole("button", { name: "Agree and continue" }).click();
  await expect
    .poll(async () => (await settings(page)).setupProfile)
    .toBe("balanced");
  await expect
    .poll(async () =>
      page.evaluate(() => window.__testAiSettings().speechModel),
    )
    .toBe("");
  const online = await page.evaluate(() => window.__testAiSettings());
  expect(online.connected).toBe(true);
  expect(online.model).toBe("");
  expect(online.backend).toBe("local");
  expect(online.quickModels).toHaveLength(1);
  expect((await settings(page)).proofreader).toBe(false);
  expect((await settings(page)).meetingAsrModel).toBe("");
});
test("download switches only on completion and cancel prevents a later switch", async ({
  page,
}) => {
  const fixture = pc();
  fixture.modelPatches[parakeet] = { state: "available", downloadedBytes: 0 };
  await open(page, fixture);
  await card(page, parakeet).click();
  await expect(
    page.getByText("Downloads here; switches when ready."),
  ).toBeVisible();
  await page
    .getByRole("region", { name: "Model download" })
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  await page.evaluate(
    (id) => window.__testLibraryProgress(id, 1.7e9, "installed"),
    parakeet,
  );
  await expect.poll(async () => await card(page, qwen).isEnabled()).toBe(true);
  expect((await settings(page)).asrModel).toBe(qwen);
  await card(page, parakeet).click();
  await expect.poll(async () => (await settings(page)).asrModel).toBe(parakeet);
});
test("insufficient disk keeps the saved model and opens the folder picker", async ({
  page,
}) => {
  const fixture = pc();
  fixture.modelPatches[parakeet] = { state: "available", downloadedBytes: 0 };
  await open(page, fixture);
  await page.evaluate(
    (id) =>
      window.__testFailModelInstall(
        id,
        "Not enough disk space: 0.4 GB free, 1.7 GB needed.",
      ),
    parakeet,
  );
  await card(page, parakeet).click();
  const sheet = page.getByRole("dialog", { name: "Not enough disk space" });
  await expect(sheet).toBeVisible();
  await shot(page, "08-not-enough-disk.png");
  expect((await settings(page)).asrModel).toBe(qwen);
  await sheet.getByRole("button", { name: "Change folder" }).click();
  await expect
    .poll(async () =>
      page.evaluate(() =>
        window.__testCalls.some(
          (call) => call.command === "plugin:dialog|open",
        ),
      ),
    )
    .toBe(true);
});
test("a completed download selects the requested model without another click", async ({
  page,
}) => {
  const fixture = pc();
  fixture.modelPatches[parakeet] = { state: "available", downloadedBytes: 0 };
  await open(page, fixture);
  await card(page, parakeet).click();
  await expect(
    page.getByText("Downloads here; switches when ready."),
  ).toBeVisible();
  expect((await settings(page)).asrModel).toBe(qwen);
  await page.evaluate(
    (id) => window.__testLibraryProgress(id, 1.7e9, "installed"),
    parakeet,
  );
  await expect.poll(async () => (await settings(page)).asrModel).toBe(parakeet);
});
test("CPU meeting speech disables Granite and marks missing AMI data", async ({
  page,
}) => {
  await open(page, pc(false), true);
  await expect(
    page.getByRole("button", { name: /^Granite Speech 4.1/ }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: /^Granite Speech 4.1/ }),
  ).toContainText("Needs a compatible NVIDIA GPU");
  // AMI scores live in each card's details now.
  await page.getByRole("button", { name: "About Same as dictation" }).click();
  await expect(
    page.getByRole("note", { name: "About Same as dictation" }),
  ).toContainText("Not measured on this kind of computer");
});
test("retrying the preview download never chooses it as the speech model", async ({
  page,
}) => {
  const id = "moonshine-tiny-streaming";
  const fixture = pc();
  fixture.modelPatches[id] = { state: "available", downloadedBytes: 0 };
  await open(page, fixture);
  await page.evaluate(
    (id) => window.__testFailModelInstall(id, "Download interrupted."),
    id,
  );
  await page.getByRole("button", { name: /Install ·/ }).click();
  await expect(page.getByRole("alert")).toContainText("Download interrupted");
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "Model download" }),
  ).toBeVisible();
  await page.evaluate(
    (id) => window.__testLibraryProgress(id, 1.7e9, "installed"),
    id,
  );
  await expect(
    page.getByRole("switch", { name: "Live preview" }),
  ).toBeEnabled();
  expect((await settings(page)).asrModel).toBe(qwen);
});
