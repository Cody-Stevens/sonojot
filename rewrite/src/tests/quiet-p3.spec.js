import { goToPage } from "./app-helpers.js";
import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import {
  installDesktopFixture,
  proofreaderCatalog,
} from "./desktop-fixture.js";

const folder = new URL("../../artifacts/redesign-p3/", import.meta.url);
const settings = (page) =>
  page.evaluate(() => window.__testSnapshot().settings);
const online = (page) =>
  page.getByRole("dialog", { name: "OpenRouter", exact: true });
const calls = (page, command) =>
  page.evaluate(
    (command) => window.__testCalls.filter((call) => call.command === command),
    command,
  );
const dual = {
  id: "test/dual",
  name: "Dual text and audio",
  contextLength: 32000,
  promptPrice: "0.000001",
  completionPrice: "0.000003",
  inputModalities: ["text", "audio"],
};
const audioOnly = {
  ...dual,
  id: "test/audio",
  name: "Audio only",
  inputModalities: ["audio"],
};
const smart = {
  id: "speakoflow-mini-q4_k_m",
  name: "Smart cleanup 0.8B",
  kind: "cleanup",
  roles: ["cleanup"],
  family: "SpeakoFlow",
  languages: ["English"],
  older: false,
  diskBytes: 540000000,
  ramMinBytes: 1140000000,
  ramMaxBytes: 1570000000,
  vramMinBytes: null,
  vramMaxBytes: null,
  memoryBasis: "Planning estimate",
  guidance: "Specialized cleanup",
  evidence: "Experimental",
  backend: "cpu",
  compatibility: "CPU",
  state: "available",
  downloadedBytes: 0,
  totalBytes: 540000000,
  error: null,
  installable: true,
  removable: true,
  usedBy: [],
  revision: "fixture",
  quantization: "Q4",
  contextTokens: 4096,
  supported: true,
};
async function open(page, options = {}, url = "/?theme=fog") {
  await installDesktopFixture(page, {
    aiCatalog: [...proofreaderCatalog, dual, audioOnly],
    aiRecommendedModels: proofreaderCatalog.slice(0, 3),
    libraryModels: [
      smart,
      {
        ...smart,
        id: "lfm2.5-2.6b",
        name: "LFM 2.6B",
        roles: ["cleanup", "notes"],
      },
    ],
    ...options,
    settings: {
      cleanupModel: "local-rules",
      localProofreader: false,
      proofreader: false,
      proofreaderModel: "anthropic/claude-opus-5.5",
      componentAcknowledgments: { "openrouter-cloud": "1" },
      ...options.settings,
    },
  });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(url, { waitUntil: "domcontentloaded" });
}
async function cleanup(page) {
  await page.getByRole("button", { name: "Cleanup", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Cleanup", exact: true }),
  ).toBeVisible();
}
async function capture(page, file) {
  await mkdir(folder, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(file, folder)) });
}

test("open dictations hide the chips and the incomplete stamp has a gap", async ({
  page,
}) => {
  await open(page);
  const stamps = page.locator(".row-text .quiet-tag");
  await expect(stamps.first()).toHaveText("NOT PASTED");
  expect(
    await stamps
      .first()
      .evaluate((element) =>
        parseFloat(getComputedStyle(element).marginInlineEnd),
      ),
  ).toBeGreaterThanOrEqual(8);
  const chips = page.getByRole("group", { name: "Dictation settings" });
  await expect(chips).toBeVisible();
  await page.locator(".row-open").first().click();
  await expect(chips).toHaveCount(0);
  await page.getByRole("button", { name: "Close dictation" }).click();
  await expect(chips).toBeVisible();
});

test("Cleanup levels, local warning and More keep their settings and capture locks", async ({
  page,
}) => {
  await open(page);
  await cleanup(page);
  await capture(page, "01-cleanup-fog.png");
  await page.getByRole("button", { name: "Format", exact: true }).click();
  expect((await settings(page)).cleanupMode).toBe("format");
  await expect(page.getByLabel("Cleanup example")).toContainText("Um, so");
  await page
    .getByRole("button", { name: "Choose a proofreader on this computer" })
    .click();
  const local = page.getByRole("dialog", { name: "Local cleanup model" });
  await local.getByRole("button", { name: /Qwen 3 4B/ }).click();
  await expect(local).toContainText("protected numbers");
  await capture(page, "02-local-warning.png");
  await local.getByRole("button", { name: "Cancel", exact: true }).click();
  expect((await settings(page)).cleanupModel).toBe("local-rules");
  await local.getByRole("button", { name: /Qwen 3 4B/ }).click();
  await local.getByRole("button", { name: "Use anyway" }).click();
  expect((await settings(page)).cleanupModel).toBe("qwen3-4b-instruct-2507");
  expect((await settings(page)).setupProfile).toBe("custom");
  await page.getByRole("button", { name: /^Advanced Written form/ }).click();
  const more = page.getByRole("dialog", { name: "Advanced", exact: true });
  await more
    .getByRole("switch", { name: "Shape by type", exact: true })
    .click();
  await more
    .getByRole("switch", { name: "Use the app I'm in", exact: true })
    .click();
  expect((await settings(page)).smartLayoutAppHint).toBe(true);
  await capture(page, "03-cleanup-more.png");
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "As heard", exact: true }).click();
  await page.getByRole("button", { name: /^Advanced Written form/ }).click();
  await expect(
    more.getByRole("switch", { name: "Shape by type", exact: true }),
  ).toBeDisabled();
  expect((await settings(page)).smartLayout).toBe(true);
  await page.keyboard.press("Escape");
  await page.evaluate(() =>
    window.__testPushSnapshot({ state: "listening", sessionId: 3 }),
  );
  await expect(
    page.getByRole("button", { name: "Clean", exact: true }),
  ).toBeDisabled();
});

test("Written form stays saved while Format and As heard disable it", async ({
  page,
}) => {
  await open(page);
  await cleanup(page);
  await page.getByRole("button", { name: "Clean", exact: true }).click();
  await page.getByRole("button", { name: /^Advanced Written form/ }).click();
  const toggle = page.getByRole("switch", {
    name: "Written form",
    exact: true,
  });
  await expect(toggle).toBeEnabled();
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  await page.keyboard.press("Escape");
  for (const name of ["Format", "As heard"]) {
    await page.getByRole("button", { name, exact: true }).click();
    await page.getByRole("button", { name: /^Advanced Written form/ }).click();
    await expect(toggle).toBeDisabled();
    await expect(toggle).not.toBeChecked();
    await page.keyboard.press("Escape");
  }
  await page.getByRole("button", { name: "Clean", exact: true }).click();
  await page.getByRole("button", { name: /^Advanced Written form/ }).click();
  await toggle.click();
  expect((await settings(page)).writtenForm).toBe(true);
});

test("browser sign-in cancels, resumes and selects cleanup without changing Notes", async ({
  page,
}) => {
  await open(page, {
    aiConnectPending: true,
    aiSettings: {
      connected: false,
      model: "test/dual",
      quickModels: [{ id: "test/dual", role: "text" }],
    },
  });
  await cleanup(page);
  await page
    .getByRole("button", { name: "Choose an online proofreader" })
    .click();
  const sheet = online(page);
  await expect(
    sheet.getByRole("button", { name: /Claude Opus/ }),
  ).toBeVisible();
  await capture(page, "04-openrouter-sign-in.png");
  await sheet.getByRole("button", { name: "Sign in with OpenRouter" }).click();
  await expect(sheet).toContainText("Finish signing in");
  await sheet.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(
    sheet.getByRole("button", { name: "Sign in with OpenRouter" }),
  ).toBeEnabled();
  expect((await settings(page)).proofreader).toBe(false);
  await sheet.getByRole("button", { name: "Sign in with OpenRouter" }).click();
  await page.evaluate(() => window.__testCompleteAiConnect());
  await expect(sheet).not.toBeVisible();
  expect((await settings(page)).proofreaderModel).toBe(
    "anthropic/claude-opus-5.5",
  );
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    model: "test/dual",
    backend: "local",
  });
  await page.getByRole("button", { name: /^None RECOMMENDED/ }).click();
  expect(await settings(page)).toMatchObject({
    proofreader: false,
    localProofreader: false,
    cleanupModel: "local-rules",
  });
});

test("key retry, catalog search and exact-model training consent preserve earlier choices", async ({
  page,
}) => {
  await open(
    page,
    {
      aiErrors: {
        connect_openrouter_key: "OpenRouter did not accept the connection.",
      },
      settings: { componentAcknowledgments: {} },
    },
    "/?theme=slate",
  );
  await cleanup(page);
  await page
    .getByRole("button", { name: "Choose an online proofreader" })
    .click();
  const sheet = online(page);
  await sheet
    .getByLabel("OpenRouter key")
    .fill("synthetic-test-key-no-network");
  await sheet.getByRole("button", { name: "Use key" }).click();
  await expect(sheet.getByRole("alert")).toContainText("did not accept");
  expect((await settings(page)).proofreader).toBe(false);
  await sheet.getByRole("button", { name: "Use key" }).click();
  const consent = page.getByRole("dialog", { name: "Use OpenRouter?" });
  await consent.getByRole("button", { name: "Cancel" }).click();
  await expect(sheet).toBeVisible();
  await expect(sheet.getByLabel("OpenRouter key")).toHaveCount(0);
  expect((await settings(page)).proofreader).toBe(false);
  await sheet.getByLabel("All OpenRouter models").fill("Muse");
  await capture(page, "05-openrouter-search-slate.png");
  await sheet.getByRole("button", { name: /Muse Spark Contributor/ }).click();
  await consent.getByRole("button", { name: "Accept online notice" }).click();
  const training = page.getByRole("dialog", {
    name: "Allow this model to train on your text?",
  });
  await training.getByRole("button", { name: "Cancel" }).click();
  expect((await settings(page)).proofreaderTrainingModels).toEqual([]);
  await sheet.getByRole("button", { name: /Muse Spark Contributor/ }).click();
  await training.getByRole("button", { name: "Allow for this model" }).click();
  await expect(sheet).not.toBeVisible();
  expect(await settings(page)).toMatchObject({
    proofreader: true,
    proofreaderTrainingModels: ["meta/muse-spark-1.3-contributor"],
  });
  expect(
    (await page.evaluate(() => window.__testAiSettings())).quickModels,
  ).toContainEqual({ id: "meta/muse-spark-1.3-contributor", role: "text" });
  await page
    .getByRole("button", { name: "Choose an online proofreader" })
    .click();
  await sheet.getByRole("button", { name: "Privacy ↗" }).click();
  expect(await calls(page, "open_openrouter_privacy")).toHaveLength(1);
  await sheet.getByRole("button", { name: "Disconnect" }).click();
  await expect(
    sheet.getByRole("button", { name: "Sign in with OpenRouter" }),
  ).toBeVisible();
});

test("one OpenRouter sheet serves Notes, meeting speech and the shared model list", async ({
  page,
}) => {
  await open(
    page,
    {
      aiSettings: {
        connected: true,
        model: "test/dual",
        quickModels: [
          { id: dual.id, role: "text" },
          { id: dual.id, role: "speech" },
        ],
      },
    },
    "/?page=setup&theme=fog",
  );
  await goToPage(page, "Meetings");
  await page.getByRole("button", { name: "Notes", exact: true }).click();
  await page.getByRole("button", { name: /Online/ }).click();
  const sheet = online(page);
  await expect(sheet).toContainText("For meeting notes");
  await sheet.getByRole("button", { name: "Remote model privacy" }).focus();
  await expect(page.getByRole("tooltip")).toContainText("sent to OpenRouter");
  await expect(
    sheet.getByRole("button", { name: /Dual text and audio/ }),
  ).toContainText("$0.0058 per 1,000 words");
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "save_settings") {
        window.__TAURI_INTERNALS__.invoke = invoke;
        throw new Error("Could not save Notes.");
      }
      return invoke(command, args);
    };
  });
  await sheet.getByRole("button", { name: /Dual text and audio/ }).click();
  await expect(sheet.getByRole("alert")).toContainText("Could not save Notes");
  expect((await settings(page)).notesModel).not.toBe("openrouter");
  expect((await page.evaluate(() => window.__testAiSettings())).backend).toBe(
    "local",
  );
  await sheet.getByRole("button", { name: /Dual text and audio/ }).click();
  await expect(sheet).not.toBeVisible();
  expect((await settings(page)).notesModel).toBe("openrouter");
  expect((await settings(page)).proofreader).toBe(false);
  await goToPage(page, "Meetings");
  await page
    .getByRole("button", { name: "Sources, speech and speakers" })
    .click();
  await page.getByRole("button", { name: /Online.*OpenRouter/ }).click();
  await expect(sheet).toContainText("For meeting speech");
  await sheet.getByLabel("All OpenRouter models").fill("Audio only");
  await sheet.getByRole("button", { name: /Audio only/ }).click();
  await expect(sheet).not.toBeVisible();
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    backend: "openrouter",
    model: dual.id,
    speechModel: audioOnly.id,
  });
  await page.getByRole("button", { name: /Online.*OpenRouter/ }).click();
  await sheet
    .getByRole("button", { name: "Manage saved models", exact: true })
    .click();
  await expect(sheet).toContainText("Your online models");
  await capture(page, "06-openrouter-shared-models.png");
  await sheet.getByLabel("All OpenRouter models").fill("Dual");
  await sheet
    .getByRole("button", { name: /Dual text and audio/ })
    .last()
    .click();
  expect(
    (await page.evaluate(() => window.__testAiSettings())).quickModels.filter(
      (model) => model.id === dual.id,
    ),
  ).toHaveLength(2);
  await sheet.getByRole("button", { name: "Disconnect" }).click();
  expect((await settings(page)).notesModel).not.toBe("openrouter");
});

test("capture during consent prevents selection; failed catalog loads offer retry", async ({
  page,
}) => {
  await open(page, {
    aiSettings: { connected: true },
    aiErrors: { list_ai_catalog: "Offline" },
  });
  await cleanup(page);
  await page
    .getByRole("button", { name: "Choose an online proofreader" })
    .click();
  const sheet = online(page);
  await expect(sheet).toContainText("Could not refresh every model list");
  await sheet.getByRole("button", { name: "Try again", exact: true }).click();
  await sheet.getByLabel("All OpenRouter models").fill("Muse");
  await sheet.getByRole("button", { name: /Muse Spark Contributor/ }).click();
  const training = page.getByRole("dialog", {
    name: "Allow this model to train on your text?",
  });
  await page.evaluate(() =>
    window.__testPushSnapshot({ state: "listening", sessionId: 4 }),
  );
  await training.getByRole("button", { name: "Allow for this model" }).click();
  expect((await settings(page)).proofreader).toBe(false);
  await expect(
    sheet.getByRole("button", { name: /Muse Spark Contributor/ }),
  ).toBeDisabled();
});

test("local downloads keep license consent, cancellation and installed selection separate", async ({
  page,
}) => {
  await open(page, {
    downloadConsentGroups: [
      {
        id: "fixture-cleanup",
        version: "1",
        models: [smart.id],
        platform: "all",
        title: "Cleanup model terms",
        summary: "Review the model terms.",
        checkbox: "I accept these model terms",
        components: [],
        links: [],
      },
    ],
  });
  await cleanup(page);
  await page
    .getByRole("button", { name: "Choose a proofreader on this computer" })
    .click();
  const local = page.getByRole("dialog", { name: "Local cleanup model" });
  await local.getByRole("button", { name: /Smart cleanup 0.8B/ }).click();
  const consent = page.getByRole("dialog", { name: "Before downloading" });
  await consent.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(await calls(page, "start_model_install")).toHaveLength(0);
  await local.getByRole("button", { name: /Smart cleanup 0.8B/ }).click();
  await consent.getByRole("checkbox").check();
  await consent.getByRole("button", { name: "Agree and continue" }).click();
  await expect(
    local.getByRole("region", { name: "Model download" }),
  ).toContainText("Downloading");
  expect((await settings(page)).cleanupModel).toBe("local-rules");
  await local.getByRole("button", { name: "Cancel", exact: true }).click();
  expect((await calls(page, "cancel_model_install"))[0].args.id).toBe(smart.id);
});
