import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture, macProfileFixture } from "./desktop-fixture.js";
import { acceptDownloadTerms } from "./app-helpers.js";
import { pc, qwen, parakeet } from "./speech-fixture.js";
const folder = new URL("../../artifacts/redesign-p6/", import.meta.url);
const calls = (page, command) =>
  page.evaluate(
    (command) => window.__testCalls.filter((call) => call.command === command),
    command,
  );
const settings = (page) =>
  page.evaluate(() => window.__testSnapshot().settings);
async function open(page, fixture = pc(), path = "/") {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await installDesktopFixture(page, {
    ...fixture,
    firstRun: false,
    settings: {
      ...fixture.settings,
      setupProfile: fixture.settings?.setupProfile ?? "balanced",
    },
  });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(path);
  await expect(
    page.getByRole("navigation", { name: "Main navigation" }),
  ).toBeVisible();
  return errors;
}
async function nav(page, tab) {
  await page.locator(`[data-nav="${tab}"]`).click();
}
async function setting(page, name) {
  await nav(page, "settings");
  await page
    .locator(".settings-link")
    .filter({ has: page.getByText(name, { exact: true }) })
    .click();
  await expect(page.getByRole("heading", { name, exact: true })).toBeVisible();
}
async function shot(page, name) {
  await mkdir(folder, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(name, folder)) });
}
for (const [name, fixture, file] of [
  ["NVIDIA", pc(), "01-nvidia-chart.png"],
  ["CPU", pc(false), "02-cpu-chart.png"],
  [
    "Mac",
    {
      ...macProfileFixture(32),
      settings: {
        ...macProfileFixture(32).settings,
        asrModel: "qwen3-asr-1.7b-q4-metal",
        setupProfile: "balanced",
      },
      modelPatches: {
        ...macProfileFixture(32).modelPatches,
        "qwen3-asr-1.7b-q4-metal": { state: "installed" },
      },
    },
    "03-mac-chart.png",
  ],
]) {
  test(`${name} plots each measured model once with nonoverlapping labels`, async ({
    page,
  }) => {
    const errors = await open(page, fixture, "/?page=setup");
    await expect(page.locator("[data-chart-model]").first()).toBeVisible();
    const ids = await page
      .locator("[data-chart-model]")
      .evaluateAll((nodes) =>
        nodes.map((node) => node.getAttribute("data-chart-model")),
      );
    expect(new Set(ids).size).toBe(ids.length);
    for (const panel of await page.locator(".speech-chart .plot").all()) {
      const boxes = await panel
        .locator("[data-chart-label]")
        .evaluateAll((nodes) =>
          nodes
            .map((node) => {
              const r = node.querySelector("text").getBoundingClientRect();
              const last = node
                .querySelector("text:last-child")
                .getBoundingClientRect();
              return { top: r.top, bottom: last.bottom };
            })
            .sort((a, b) => a.top - b.top),
        );
      for (let i = 1; i < boxes.length; i++)
        expect(boxes[i].top).toBeGreaterThan(boxes[i - 1].bottom);
    }
    if (name === "NVIDIA") {
      await expect(
        page
          .getByRole("region", { name: "GPU memory chart" })
          .locator("[data-chart-model]"),
      ).toHaveCount(1);
      await expect(
        page
          .getByRole("region", { name: "CPU system memory chart" })
          .locator("[data-chart-model]"),
      ).toHaveCount(1);
    }
    if (name === "Mac") {
      await expect(
        page.getByRole("region", { name: "GPU memory chart" }),
      ).toContainText("unified memory");
      await expect(
        page
          .getByRole("region", { name: "CPU system memory chart" })
          .locator("[data-chart-model]"),
      ).toHaveCount(0);
    }
    await shot(page, file);
    expect(errors).toEqual([]);
  });
}
test("Words adds immediately, edits aliases, protects drafts and handles suggestions", async ({
  page,
}) => {
  const errors = await open(
    page,
    {
      ...pc(),
      vocabulary: [
        {
          id: "north",
          heard: "north wind",
          written: "Northwind",
          enabled: true,
          aliases: ["north wind"],
          context: ["order"],
          scope: null,
        },
      ],
      vocabularySuggestions: [
        {
          heard: "tax sheet",
          written: "Tacksheet",
          reason: "A recurring phrase",
        },
      ],
    },
    "/?page=vocabulary",
  );
  await page
    .getByRole("textbox", { name: "Add a word", exact: true })
    .fill("omar key -> Omarchy");
  await page
    .getByRole("textbox", { name: "Add a word", exact: true })
    .press("Enter");
  await expect
    .poll(async () => (await calls(page, "save_vocabulary")).length)
    .toBe(1);
  expect(
    (await calls(page, "save_vocabulary"))[0].args.entries.at(-1).aliases,
  ).toEqual(["omar key"]);
  await page.getByRole("searchbox", { name: "Search words" }).fill("omar");
  await page.getByRole("button", { name: /omar key.*Omarchy/ }).click();
  await page
    .getByRole("textbox", { name: "Also hears" })
    .fill("omar key, omar kee");
  await page
    .getByRole("textbox", { name: "Near words" })
    .fill("linux, desktop");
  await shot(page, "04-words-editor.png");
  await nav(page, "settings");
  await expect(
    page.getByRole("dialog", { name: "Leave these changes?" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Keep working", exact: true }).click();
  await page.getByRole("button", { name: "Done", exact: true }).click();
  expect(
    (await calls(page, "save_vocabulary")).at(-1).args.entries.at(-1).context,
  ).toEqual(["linux", "desktop"]);
  await page.getByRole("button", { name: "Suggested", exact: true }).click();
  await page.getByRole("button", { name: "Review recent dictations" }).click();
  await page.getByRole("button", { name: "Add", exact: true }).click();
  expect(
    (await calls(page, "save_vocabulary")).at(-1).args.entries.at(-1).written,
  ).toBe("Tacksheet");
  await page.getByRole("button", { name: "Learned", exact: true }).click();
  await expect(
    page.getByRole("switch", {
      name: "Learn from my corrections on this device",
    }),
  ).toBeVisible();
  await page.getByText("More learning options", { exact: true }).click();
  await page
    .getByRole("button", { name: "Reset learning…", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toContainText("Manual vocabulary");
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(await calls(page, "reset_learning")).toHaveLength(0);
  expect(errors).toEqual([]);
});
test("Settings doors share stage screens and reset names resolve online catalogs", async ({
  page,
}) => {
  const fixture = pc();
  await open(page, {
    ...fixture,
    settings: {
      ...fixture.settings,
      asrModel: parakeet,
      proofreader: true,
      proofreaderModel: "provider/cleanup",
      localProofreader: false,
    },
    aiSettings: {
      connected: true,
      backend: "openrouter",
      model: "unknown/notes",
      speechModel: "unknown/speech",
    },
    aiCatalog: [
      {
        id: "provider/cleanup",
        name: "Polished text",
        contextLength: 1000,
        promptPrice: null,
        completionPrice: null,
      },
    ],
  });
  await setting(page, "Dictation");
  await expect(page.locator(".stage-diff").first()).toBeVisible();
  await expect(page.locator(".settings-link")).toContainText([
    "Shortcut and mic",
    "Speech",
    "Cleanup",
    "Paste and history",
  ]);
  await shot(page, "05-settings-dictation.png");
  await page
    .getByRole("button", { name: "Back to recommended", exact: true })
    .click();
  const sheet = page.getByRole("dialog", { name: "Back to recommended setup" });
  await expect(sheet).toContainText("Polished text");
  await expect(sheet).toContainText("Selected OpenRouter model");
  await expect(sheet).not.toContainText("provider/");
  await expect(sheet).not.toContainText("unknown/");
  await sheet.getByRole("button", { name: "Use recommended setup" }).click();
  await acceptDownloadTerms(page);
  await expect.poll(async () => (await settings(page)).asrModel).toBe(qwen);
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    connected: true,
    backend: "local",
    model: "",
    speechModel: "",
  });
  await page
    .locator(".settings-link")
    .filter({ has: page.getByText("Speech", { exact: true }) })
    .click();
  await expect(
    page.getByRole("heading", { name: "Speech", exact: true }),
  ).toBeVisible();
  await setting(page, "Meetings");
  await page
    .locator(".settings-link")
    .filter({ has: page.getByText("Speech and speakers", { exact: true }) })
    .click();
  await expect(
    page.getByRole("group", { name: "Meeting sources" }),
  ).toBeVisible();
});
test("Look uses paper switches; memory and update controls save and honor capture locks", async ({
  page,
}) => {
  await open(page);
  await setting(page, "Look and startup");
  await page.getByRole("button", { name: "Slate", exact: true }).click();
  await page
    .getByRole("switch", { name: "Start at login", exact: true })
    .click();
  await page
    .getByRole("switch", { name: "Floating control at start", exact: true })
    .click();
  expect(await settings(page)).toMatchObject({
    theme: "slate",
    launchAtLogin: true,
    startTabHidden: true,
  });
  const relief = await page
    .getByRole("switch", { name: "Start at login", exact: true })
    .evaluate((node) => ({
      slot: getComputedStyle(node).boxShadow,
      tab: getComputedStyle(node.firstElementChild).boxShadow,
    }));
  expect(relief.slot).toContain("inset");
  expect(relief.tab).not.toBe("none");
  await shot(page, "06-look-slate.png");
  await setting(page, "Memory");
  await page
    .getByRole("button", { name: "Load one at a time", exact: true })
    .click();
  await page.getByRole("button", { name: "Immediately", exact: true }).click();
  await page.getByRole("button", { name: "CPU only", exact: true }).click();
  expect(await settings(page)).toMatchObject({
    memoryMode: "sequential",
    modelRetention: "immediately",
    textAcceleration: "cpu",
  });
  await page.evaluate(() => window.__testPushSnapshot({ state: "listening" }));
  await expect(
    page.getByRole("button", { name: "Keep all ready", exact: true }),
  ).toBeDisabled();
  await page.evaluate(() => window.__testPushSnapshot({ state: "idle" }));
  await setting(page, "Updates");
  await page.getByRole("button", { name: "Nightly", exact: true }).click();
  expect((await settings(page)).updateChannel).toBe("nightly");
  await page.evaluate(() =>
    window.__testPushSnapshot({
      meetingRecording: true,
      update: {
        ...window.__testSnapshot().update,
        status: { state: "ready", version: "9.0", notes: null },
        restartBlockedReason: "Stop the meeting before restarting.",
      },
    }),
  );
  await expect(
    page.locator(".quiet-settings").getByRole("button", { name: /Restart/ }),
  ).toBeDisabled();
});
test("On disk rechecks removal after confirmation and changes the real folder", async ({
  page,
}) => {
  await open(page);
  await setting(page, "On disk");
  await page
    .getByRole("searchbox", { name: "Search models on disk" })
    .fill("Qwen3-ASR 1.7B · GPU");
  const row = page.locator(`.disk-entry[data-model-id="${qwen}"]`);
  await row.locator(".disk-row").click();
  await shot(page, "07-on-disk.png");
  await row.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(page.getByRole("dialog")).toContainText("Speech will switch");
  await page.evaluate(() =>
    window.__testPushSnapshot({ meetingRecording: true }),
  );
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Remove", exact: true })
    .click();
  expect(await calls(page, "remove_model")).toHaveLength(0);
  await page.evaluate(() =>
    window.__testPushSnapshot({ meetingRecording: false }),
  );
  await row.getByRole("button", { name: "Remove", exact: true }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Remove", exact: true })
    .click();
  await expect
    .poll(async () => (await calls(page, "remove_model")).length)
    .toBe(1);
  expect((await settings(page)).asrModel).not.toBe(qwen);
  await page.getByRole("searchbox", { name: "Search models on disk" }).fill("");
  await page.getByRole("button", { name: "Change", exact: true }).click();
  expect(await calls(page, "locate_models")).toHaveLength(1);
  await page.getByRole("button", { name: "Not offered", exact: true }).click();
  await expect(page.locator(".disk-list")).toContainText(
    /Outside the recommended|requires|Replaced/,
  );
});
test("Help copies the installed CLI, previews a report and replays consent", async ({
  page,
}) => {
  await open(
    page,
    {
      ...pc(),
      settings: { ...pc().settings, meetingConsentAcknowledged: true },
    },
    "/?page=about",
  );
  await expect(
    page.getByRole("heading", { name: "Help", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Copy path", exact: true }).click();
  expect(await page.evaluate(() => window.__testClipboard)).toContain(
    "Eavesdropper Next/sonojot-cli.exe",
  );
  await page.getByRole("button", { name: /Report a problem See/ }).click();
  await expect(
    page.getByRole("dialog", { name: "Report a problem", exact: true }),
  ).toBeVisible();
  await page.getByText("Show what will be sent", { exact: true }).click();
  await expect(page.locator(".report-preview")).toContainText("requestId");
  await expect(page.locator(".report-error")).toHaveCount(0);
  await shot(page, "08-help-report.png");
  expect(await calls(page, "send_report")).toHaveLength(0);
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Run again", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "SonoJot tour" }),
  ).toBeVisible();
  expect((await settings(page)).meetingConsentAcknowledged).toBe(false);
});

test("the last usable speech model is protected and failed replacement saves never remove it", async ({
  page,
}) => {
  await open(page);
  await page.evaluate(async (id) => {
    const library =
      await window.__TAURI_INTERNALS__.invoke("get_model_library");
    for (const model of library.models.filter(
      (model) => model.kind === "asr" && model.id !== id,
    ))
      window.__testPatchModel(model.id, {
        state: "unavailable",
        supported: false,
        installable: false,
      });
  }, qwen);
  await setting(page, "On disk");
  const row = page.locator(`.disk-entry[data-model-id="${qwen}"]`);
  await row.locator(".disk-row").click();
  await expect(
    row.getByRole("button", { name: "Remove", exact: true }),
  ).toBeDisabled();
  await expect(row).toContainText("install another speech model first");
  await page.evaluate((id) => {
    window.__testPatchModel(id, {
      state: "installed",
      supported: true,
      installable: true,
    });
    window.dispatchEvent(new Event("focus"));
  }, parakeet);
  await expect(
    row.getByRole("button", { name: "Remove", exact: true }),
  ).toBeEnabled();
  await row.getByRole("button", { name: "Remove", exact: true }).click();
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "save_settings")
        throw new Error("Could not save the replacement.");
      return invoke(command, args);
    };
  });
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Remove", exact: true })
    .click();
  await expect(page.getByRole("alert")).toBeVisible();
  expect(await calls(page, "remove_model")).toHaveLength(0);
  expect((await settings(page)).asrModel).toBe(qwen);
});
test("online Notes leave the saved local notes package independently removable", async ({
  page,
}) => {
  await open(page, {
    ...pc(),
    aiSettings: { backend: "openrouter", connected: true, model: "mock/text" },
  });
  await setting(page, "On disk");
  const row = page.locator(
    '.disk-entry[data-model-id="qwen3-4b-instruct-2507"]',
  );
  await row.locator(".disk-row").click();
  await row.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(page.getByRole("dialog")).not.toContainText("will switch");
  const before = (await calls(page, "save_settings")).length;
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Remove", exact: true })
    .click();
  await expect
    .poll(async () => (await calls(page, "remove_model")).length)
    .toBe(1);
  expect(await calls(page, "save_settings")).toHaveLength(before + 1);
  expect((await settings(page)).notesModel).toBe("lfm2.5-1.2b");
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    backend: "openrouter",
    model: "mock/text",
  });
});

test("punctuation download and other local Notes remain reachable after model management moves", async ({
  page,
}) => {
  const fixture = pc();
  await open(page, {
    ...fixture,
    modelPatches: {
      ...fixture.modelPatches,
      "sonojot-punct-v3": { state: "available", downloadedBytes: 0 },
    },
    settings: { ...fixture.settings, sonojotPunctuation: true },
  });
  const before = (await settings(page)).asrModel;
  await setting(page, "Dictation");
  await page
    .locator(".settings-link")
    .filter({ has: page.getByText("Cleanup", { exact: true }) })
    .click();
  await page.getByRole("button", { name: /More Punctuation/ }).click();
  await page
    .getByRole("button", { name: "Install punctuation model", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "On disk", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Install punctuation model", exact: true })
    .click();
  await expect
    .poll(async () => (await calls(page, "start_model_install")).length)
    .toBe(1);
  await page
    .getByRole("region", { name: "Model download" })
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  expect((await settings(page)).asrModel).toBe(before);
  await setting(page, "Meetings");
  await page
    .locator(".settings-link")
    .filter({ has: page.getByText("Notes", { exact: true }) })
    .click();
  await page.locator(".writers .quiet-option").first().click();
  await page
    .getByRole("button", { name: "Other local models", exact: true })
    .click();
  await expect(
    page.getByRole("dialog", { name: "Local notes model" }),
  ).toContainText("Mock Cleanup 2B (NVIDIA CUDA)");
});
