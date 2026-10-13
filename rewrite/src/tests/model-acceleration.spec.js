import { expect, test } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";

test("llama Qwen Q8 CUDA appears in existing controls without changing selection", async ({
  page,
}, testInfo) => {
  await installDesktopFixture(page);
  await page.goto("/?page=setup&theme=fog");
  await page.evaluate(() => {
    window.__testPatchModel("qwen3-asr-1.7b-fp16-cuda", {
      id: "qwen3-asr-1.7b-q8-cuda",
      name: "Qwen3-ASR 1.7B (Q8 CUDA)",
      state: "installed",
      removable: true,
      backend: "llama.cpp b10900 CUDA 13.3 (Qwen3-ASR Q8_0 decoder + Q8_0 audio encoder; VAD CPU)",
      guidance: "Multilingual · NVIDIA CUDA · llama.cpp",
      diskBytes: 2520744288,
      ramMinBytes: 2600 * 1048576,
      ramMaxBytes: 3400 * 1048576,
      vramMinBytes: 3200 * 1048576,
      vramMaxBytes: 4200 * 1048576,
      memoryBasis: "Observed 2026-09-28 on RTX 5070 with the pinned llama-server",
    });
    window.dispatchEvent(new Event("focus"));
  });
  const before = await page.evaluate(() => window.__testSnapshot().settings);
  const library = page.getByRole("region", { name: "Model library" });
  await library.getByRole("searchbox").fill("Q8 CUDA");
  const row = library.locator('[data-model-id="qwen3-asr-1.7b-q8-cuda"]');
  await expect(row).toContainText("Qwen3-ASR 1.7B (Q8 CUDA)");
  await expect(row).toContainText("NVIDIA CUDA");
  expect(await page.evaluate(() => window.__testSnapshot().settings)).toEqual(before);
  await page.locator("#model-asr").click();
  await page.getByRole("dialog").getByRole("option", { name: /Q8 CUDA/ }).click();
  await expect(page.locator("#setup-asr")).toContainText("Qwen3-ASR 1.7B (Q8 CUDA)");
  await expect(page.locator("#setup-asr")).toContainText("NVIDIA CUDA");
  await expect(page.locator("#setup-asr")).not.toContainText("CPU only");
  await page.locator("#setup-asr").scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath("llama-qwen-q8-cuda.png") });
});

test("main loading status and GPU badge follow recognition phases without refocusing", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=setup&theme=fog");
  await page.evaluate(() => {
    window.__testPatchModel("qwen3-asr-1.7b-fp16-cuda", { state: "installed" });
    window.dispatchEvent(new Event("focus"));
  });
  await page.locator("#model-asr").click();
  await page
    .getByRole("dialog")
    .getByRole("option", { name: /FP16 CUDA/ })
    .click();
  await expect(page.locator("#setup-asr")).toContainText("not loaded");
  await page.evaluate(() =>
    window.__testPushSnapshot({
      state: "loading",
      sessionId: 20,
      recognitionProgress: { stage: "loading", elapsedMs: 12000 },
    }),
  );
  await expect(
    page.getByText("Recording · loading model 12s", { exact: true }),
  ).toBeVisible();
  await page.evaluate(() => {
    window.__testAsrRuntime({
      modelId: "qwen3-asr-1.7b-fp16-cuda",
      backend: "cuda",
      compatibility: "NVIDIA CUDA",
      providerRequested: "cuda",
      providerInitialized: "cuda",
      deviceIndex: 0,
      executionProof: null,
    });
    window.__testPushSnapshot({
      state: "listening",
      recognitionProgress: { stage: "transcribing", elapsedMs: 0 },
    });
  });
  await expect(page.locator("#setup-asr")).toContainText("NVIDIA CUDA loaded");
  await page.evaluate(() => {
    window.__testAsrRuntime({
      modelId: null,
      backend: null,
      compatibility: null,
      providerRequested: "cuda",
      providerInitialized: "cpu",
      deviceIndex: null,
      executionProof: null,
    });
    window.__testPushSnapshot({
      state: "idle",
      sessionId: null,
      recognitionProgress: null,
    });
  });
  await expect(page.locator("#setup-asr")).toContainText("not loaded");
  await expect(page.getByText("Idle", { exact: true })).toBeVisible();
});

test("model controls show capability, preserve settings, and report live GPU or CPU fallback", async ({
  page,
}, testInfo) => {
  await installDesktopFixture(page);
  await page.goto("/?page=setup&theme=fog");
  // Speech options stay explicit about CPU execution.
  await expect(page.locator("#setup-asr")).toContainText("CPU only");
  const performance = page.getByRole("region", { name: "Performance" });
  const control = performance.getByRole("combobox", {
    name: "Text model processing",
  });
  const status = page
    .locator("#setup-cleanup")
    .locator('.model-acceleration[data-model-id="lfm2.5-1.2b"]');
  await expect(status).toContainText("NVIDIA CUDA available");
  await expect(status).not.toContainText("GPU accelerated");
  const before = await page.evaluate(() => window.__testSnapshot().settings);
  await control.selectOption("cpu");
  await expect(status).toContainText("CPU selected");
  expect(
    await page.evaluate(() => window.__testSnapshot().settings),
  ).toMatchObject({ ...before, textAcceleration: "cpu" });
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__testRuntime = {
      pid: 1,
      modelId: "lfm2.5-1.2b",
      backend: "cuda",
      loading: false,
      gpuLayers: 37,
      totalLayers: 37,
      fallbackReason: null,
    };
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      const result = await invoke(command, args);
      return command === "get_model_acceleration"
        ? { ...result, textRuntime: window.__testRuntime }
        : result;
    };
  });
  await control.selectOption("auto");
  await expect(status).toContainText("GPU accelerated · NVIDIA CUDA");
  await expect(status).not.toContainText("37/37 layers on GPU");
  await status.scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath("model-gpu.png") });
  await page.evaluate(() => {
    window.__testRuntime = {
      ...window.__testRuntime,
      backend: "cpu",
      gpuLayers: 0,
      fallbackReason: "CUDA could not start; using CPU.",
    };
    window.dispatchEvent(new Event("focus"));
  });
  await expect(status).toContainText("CPU · model loaded");
  await expect(status).not.toContainText("CUDA could not start");
});

test("text acceleration preserves metal wording on mac instead of all-platform cuda", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=setup&theme=fog");
  await page.evaluate(() =>
    window.__testPushSnapshot({
      capabilities: {
        autoPaste: true,
        systemAudio: true,
        globalShortcut: true,
        platform: "macos",
      },
    }),
  );
  const performance = page.getByRole("region", { name: "Performance" });
  await expect(
    performance.getByRole("combobox", { name: "Text model processing" }),
  ).toContainText("Metal when available");
  await expect(performance).not.toContainText(
    "Metal failure falls back to CPU",
  );
});

test("resident CUDA runtime shows initialized label without claiming measured execution", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=setup&theme=fog");
  // The fixture installs its hooks on navigation; set the resident override
  // before selecting the GPU model so the row mounts against it.
  await page.evaluate(() =>
    window.__testAsrRuntime({
      modelId: "qwen3-asr-1.7b-fp16-cuda",
      backend: "sherpa-onnx-1.13.8-cuda13 test backend",
      compatibility: "Windows x64 NVIDIA CUDA 13.x test compat",
      providerRequested: "cuda",
      providerInitialized: "cuda",
      deviceIndex: 0,
      executionProof: null,
    }),
  );
  await page.evaluate(() => {
    window.__testPatchModel("qwen3-asr-1.7b-fp16-cuda", { state: "installed" });
    window.__testPatchModel("qwen3-asr-1.7b", { state: "installed" });
    window.dispatchEvent(new Event("focus"));
  });
  await page.locator("#model-asr").click();
  await page
    .getByRole("dialog")
    .getByRole("option", { name: /FP16 CUDA/ })
    .click();
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  const gpuRow = page.locator("#setup-asr");
  await expect(gpuRow).toContainText("NVIDIA CUDA loaded");
  await expect(gpuRow).not.toContainText("not measured");
  await expect(gpuRow).not.toContainText("GPU accelerated");
  // CPU Qwen int8 stays CPU only even with a CUDA resident elsewhere.
  await page.locator("#model-asr").click();
  await page
    .getByRole("dialog")
    .getByRole("option", { name: /CPU int8/ })
    .click();
  await expect(page.locator("#setup-asr")).toContainText("CPU only");
});

test("notes model summary is one line without duplicate controls", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=meetings&theme=slate");
  await page.getByRole("button", { name: /Design review/ }).click();
  await page.getByRole("tab", { name: "Options", exact: true }).click();
  await expect(page.getByLabel("Notes model")).toHaveCount(0);
  await expect(page.getByLabel("Text model processing")).toHaveCount(0);
  await expect(page.getByText("Notes · Qwen 3 4B · CPU")).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Manage .* in Setup/ }),
  ).toHaveCount(0);
});
