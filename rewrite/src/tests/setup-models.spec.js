import { expect, test } from "@playwright/test";
import {
  goToPage,
  openApp,
  openModelLibrary,
  openPerformance,
} from "./app-helpers.js";

// The models folder's Change… button, in the Performance and storage fold.
const changeFolder = (page) =>
  page
    .getByRole("group", { name: "Models folder" })
    .getByRole("button", { name: "Change…", exact: true });

test.beforeEach(async ({ page }) => {
  await openApp(page, {}, "/?page=setup&theme=fog");
  await expect(
    page.getByRole("button", { name: "Cleanup model", exact: true }),
  ).toBeEnabled();
});

const settings = (page) =>
  page.evaluate(() => window.__testSnapshot().settings);
const saves = (page) =>
  page.evaluate(() =>
    window.__testCalls.filter((call) => call.command === "save_settings"),
  );
async function openPicker(page, role, search) {
  await page
    .getByRole("button", { name: `${role} model`, exact: true })
    .click();
  const picker = page.getByRole("dialog", {
    name: `${role} models`,
    exact: true,
  });
  if (search) await picker.getByRole("searchbox").fill(search);
  return picker;
}

test("missing choice installs independently and completion requires explicit selection", async ({
  page,
}) => {
  const before = await settings(page);
  const picker = await openPicker(
    page,
    "Cleanup",
    "Mock Cleanup 2B (NVIDIA CUDA)",
  );
  await picker.getByRole("option").click();
  expect(await settings(page)).toEqual(before);
  expect(await saves(page)).toHaveLength(0);
  await expect(
    page.getByRole("button", { name: "Cleanup model", exact: true }),
  ).toHaveText("LFM 2.5");
  await picker.getByRole("button", { name: "Install", exact: true }).click();
  const download = page.getByRole("region", { name: "Model download" });
  await expect(download).toContainText("Downloading");
  await expect(download.getByRole("progressbar")).toHaveAttribute(
    "value",
    /\d+/,
  );
  await expect(
    page.getByRole("button", { name: "Cleanup model", exact: true }),
  ).toBeDisabled();
  await openPerformance(page);
  await expect(
    page.getByRole("combobox", { name: "Text model processing" }),
  ).toBeDisabled();
  await expect(changeFolder(page)).toBeDisabled();
  await page.evaluate(() => {
    window.__testLibraryProgress(
      "mock-cleanup-2b-cuda",
      2147483648,
      "verifying",
    );
    window.dispatchEvent(new Event("focus"));
  });
  await expect(download).toContainText("Verifying");
  await goToPage(page, "Dictation");
  await page.evaluate(() =>
    window.__testLibraryProgress(
      "mock-cleanup-2b-cuda",
      2147483648,
      "installed",
    ),
  );
  await goToPage(page, "Models");
  await expect(download).toContainText("Installed");
  expect(await settings(page)).toEqual(before);
  const completed = await openPicker(
    page,
    "Cleanup",
    "Mock Cleanup 2B (NVIDIA CUDA)",
  );
  await completed.getByRole("option").click();
  await expect(
    page.getByRole("button", { name: "Cleanup model", exact: true }),
  ).toContainText("Mock Cleanup 2B");
  expect(await settings(page)).toEqual({
    ...before,
    cleanupModel: "mock-cleanup-2b-cuda",
  });
  expect(await saves(page)).toHaveLength(1);
});

test("library removal is independent, protects selected models and rechecks after confirmation", async ({
  page,
}) => {
  const library = await openModelLibrary(page);
  const before = await settings(page);
  await expect(
    library.getByRole("button", { name: "Remove LFM 2.5", exact: true }),
  ).toBeDisabled();
  await library.getByRole("searchbox").fill("Mock Cleanup 2B (CPU)");
  const remove = library.getByRole("button", {
    name: "Remove Mock Cleanup 2B (CPU)",
    exact: true,
  });
  await remove.click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  await expect(remove).toBeEnabled();
  await remove.click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Remove", exact: true })
    .click();
  await expect(remove).toHaveCount(0);
  expect(await settings(page)).toEqual(before);
  expect(await saves(page)).toHaveLength(0);
  await library.getByRole("tab", { name: "Get more", exact: true }).click();
  await expect(
    library.getByRole("button", {
      name: "Install Mock Cleanup 2B (CPU)",
      exact: true,
    }),
  ).toBeVisible();

  await page.evaluate(() => {
    window.__testPatchModel("mock-cleanup-2b-cpu", { state: "installed" });
    window.dispatchEvent(new Event("focus"));
  });
  await library.getByRole("tab", { name: "Installed", exact: true }).click();
  await remove.click();
  await page.evaluate(() => {
    const snapshot = window.__testSnapshot();
    window.__testPushSnapshot({
      settings: { ...snapshot.settings, notesModel: "mock-cleanup-2b-cpu" },
    });
  });
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Remove", exact: true })
    .click();
  await expect(remove).toBeDisabled();
  expect(
    await page.evaluate(() =>
      window.__testCalls.filter((call) => call.command === "remove_model"),
    ),
  ).toHaveLength(1);
});

test("keyboard browsing, unavailable models and failed saves preserve current selection", async ({
  page,
}) => {
  const before = await settings(page);
  await page.evaluate(() => {
    window.__testPatchModel("mock-cleanup-old-1b", {
      state: "unsupported",
      installable: false,
    });
    window.dispatchEvent(new Event("focus"));
  });
  const trigger = page.getByRole("button", {
    name: "Cleanup model",
    exact: true,
  });
  await trigger.focus();
  await page.keyboard.press("ArrowDown");
  const picker = page.getByRole("dialog", {
    name: "Cleanup models",
    exact: true,
  });
  await expect(picker.getByRole("searchbox")).toBeFocused();
  await picker.getByRole("searchbox").fill("older");
  await page.keyboard.press("ArrowUp");
  await expect(picker.getByRole("option")).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(
    picker.getByRole("button", { name: "Unavailable", exact: true }),
  ).toBeDisabled();
  expect(await settings(page)).toEqual(before);
  await page.keyboard.press("Escape");
  await expect(trigger).toBeFocused();
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = (command, args) => {
      if (command === "save_settings")
        throw new Error("Mock settings write failed");
      return invoke(command, args);
    };
  });
  const installed = await openPicker(page, "Cleanup", "Formatting");
  await installed.getByRole("searchbox").press("Tab");
  await expect(installed.getByRole("option")).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(trigger).toHaveText("LFM 2.5");
  expect(await settings(page)).toEqual(before);
  await expect(page.getByRole("alert")).toContainText(
    "The setting could not be saved",
  );
});

test("active processing locks model changes, while library tabs, filtering and paging work", async ({
  page,
}) => {
  const library = await openModelLibrary(page);
  await expect(library.locator(".entry")).toHaveCount(5);
  await library.getByRole("button", { name: "Next models" }).click();
  await expect(library.locator(".entry")).toHaveCount(2);
  await library.getByRole("tab", { name: "Installed", exact: true }).focus();
  await page.keyboard.press("ArrowRight");
  await expect(
    library.getByRole("tab", { name: "Get more", exact: true }),
  ).toBeFocused();
  await library
    .getByRole("combobox", { name: "Filter model library" })
    .selectOption("notes");
  await expect(library.locator(".entry")).toHaveCount(1);
  await page.evaluate(() => {
    window.__testPatchModel("lfm2.5-1.2b", { state: "busy" });
    window.dispatchEvent(new Event("focus"));
  });
  await expect(
    page.getByRole("button", { name: "Speech model", exact: true }),
  ).toBeDisabled();
  await expect(
    library.getByRole("button", { name: /^Install / }),
  ).toBeDisabled();
  await openPerformance(page);
  await expect(
    page.getByRole("combobox", { name: "Memory use" }),
  ).toBeDisabled();
});

test("GPU budget preserves proposed demand after CPU fallback and marks unknown intervals", async ({
  page,
}) => {
  await page.evaluate(() => {
    window.__testPatchModel("qwen3-asr-1.7b-fp16-cuda", {
      state: "installed",
      ramMinBytes: 12e9,
      ramMaxBytes: 18e9,
      vramMinBytes: 8e9,
      vramMaxBytes: 12e9,
    });
    window.dispatchEvent(new Event("focus"));
  });
  const picker = await openPicker(page, "Speech", "FP16");
  await picker.getByRole("option").click();
  const gpu = page.locator('[data-budget="GPU VRAM"]');
  await expect(gpu).toContainText("exceeds capacity");
  await expect(gpu).toHaveAttribute("data-unknown", "false");
  const plan = await gpu.getByRole("img").getAttribute("aria-label");
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      const result = await invoke(command, args);
      return command === "get_model_acceleration"
        ? {
            ...result,
            textRuntime: {
              pid: 1,
              modelId: "lfm2.5-1.2b",
              backend: "cpu",
              loading: false,
              gpuLayers: 0,
              totalLayers: 24,
              fallbackReason: "Proposed combination exceeds VRAM.",
            },
          }
        : result;
    };
    window.dispatchEvent(new Event("focus"));
  });
  await expect(page.locator("#setup-cleanup")).toContainText(
    "CPU · model loaded",
  );
  await expect(gpu.getByRole("img")).toHaveAttribute("aria-label", plan);
  await page.evaluate(() => {
    window.__testPatchModel("qwen3-asr-1.7b-fp16-cuda", {
      vramMinBytes: null,
      vramMaxBytes: null,
    });
    window.dispatchEvent(new Event("focus"));
  });
  await expect(gpu).toHaveAttribute("data-unknown", "true");
  await expect(gpu).toContainText("Estimate unknown");
  await expect(gpu.locator('.bar [data-segment="Headroom"]')).toHaveCount(0);
  await expect(gpu.locator(".bar .unknown")).toBeVisible();
});

for (const [theme, width] of [
  ["fog", 1220],
  ["slate", 390],
]) {
  test(`setup and picker fit ${theme} at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 860 });
    if (theme !== "fog") await page.goto(`/?page=setup&theme=${theme}`);
    await expect(page.locator(".studio")).toHaveAttribute("data-theme", theme);
    await expect(
      page.getByRole("button", { name: "Cleanup model", exact: true }),
    ).toBeEnabled();
    const models = page.getByRole("region", { name: "Speech and text models" });
    const trigger = page.getByRole("button", {
      name: "Cleanup model",
      exact: true,
    });
    await expect(trigger).toHaveCSS(
      "background-color",
      theme === "slate" ? "rgb(59, 68, 58)" : "rgb(255, 253, 247)",
    );
    await expect(trigger).toHaveCSS("opacity", "1");
    await expect(models.locator('[data-budget="HOST RAM"]')).toBeVisible();
    await expect(models.locator('[data-budget="GPU VRAM"]')).toBeVisible();
    await openPerformance(page);
    expect(
      await changeFolder(page).evaluate(
        (element) => getComputedStyle(element).fontSize,
      ),
    ).toBe("18px");
    await page.screenshot({
      path: testInfo.outputPath(`setup-${theme}-${width}.png`),
    });
    const picker = await openPicker(page, "Cleanup", "NVIDIA");
    await picker.getByRole("option").filter({ hasText: "Not installed" }).first().click();
    await expect(
      picker.getByRole("button", { name: "Install", exact: true }),
    ).toBeVisible();
    const bounds = await picker.boundingBox();
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(width);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(860);
    expect(
      await models.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
    expect(
      await picker.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
    expect(
      await picker
        .getByRole("searchbox")
        .evaluate((element) => getComputedStyle(element).fontSize),
    ).toBe("18px");
    await page.screenshot({
      path: testInfo.outputPath(`picker-${theme}-${width}.png`),
    });
  });
}
