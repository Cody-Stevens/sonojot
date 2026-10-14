import { expect, test } from "@playwright/test";

test("memory error stays visible after the pointer leaves the tab", async ({
  page,
}) => {
  await screenTab(page);
  await page.evaluate(() =>
    window.__tabPush({
      state: "error",
      error:
        "Memory unavailable: Selected CUDA device has too little free memory. Choose a smaller model.",
      tabExpanded: true,
      tabVisible: true,
      tabTemporary: false,
    }),
  );
  await page.mouse.move(1, 1);
  await expect(page.getByRole("alert")).toContainText("Not enough memory");
  await page.waitForTimeout(600);
  await expect(page.getByRole("alert")).toBeVisible();
  expect(await page.evaluate(() => window.__tabState().tabExpanded)).toBe(true);
});

test("tab distinguishes cold loading, active listening and stopped draining", async ({
  page,
}) => {
  await screenTab(page);
  await page.evaluate(() =>
    window.__tabPush({
      state: "loading",
      tabExpanded: true,
      sessionId: 8,
      recognitionProgress: { stage: "loading", elapsedMs: 24000 },
    }),
  );
  await expect(page.locator(".tab-control-row [role='status']")).toContainText(
    "Recording · loading model 24s",
  );
  await page.evaluate(() =>
    window.__tabPush({
      state: "finishing",
      recognitionProgress: { stage: "loading", elapsedMs: 25000 },
    }),
  );
  await expect(page.locator(".tab-control-row [role='status']")).toContainText(
    "Loading speech model · 25s",
  );
  await page.evaluate(() =>
    window.__tabPush({
      state: "listening",
      recognitionProgress: { stage: "transcribing", elapsedMs: 0 },
    }),
  );
  await expect(page.locator(".tab-control-row [role='status']")).toContainText(
    "Listening",
  );
});

test("copied status leaves every idle action inside the sheet", async ({
  page,
}, testInfo) => {
  await screenTab(page);
  await page.evaluate(() =>
    window.__tabPush({
      state: "idle",
      tabExpanded: true,
      tabTemporary: true,
      deliveryNotice: "Last dictation copied.",
    }),
  );
  await page.evaluate(() => document.fonts.ready);
  const sheet = await page.locator(".tab-sheet").boundingBox();
  for (const button of await page.locator(".tab-actions button").all()) {
    const box = await button.boundingBox();
    expect(box.x).toBeGreaterThanOrEqual(sheet.x);
    expect(box.x + box.width).toBeLessThanOrEqual(sheet.x + sheet.width - 10);
  }
  await page
    .locator(".screen-tab")
    .screenshot({ path: testInfo.outputPath("copied-status.png") });
});

test("live preview has clearance above and below its text", async ({
  page,
}, testInfo) => {
  await screenTab(page);
  await page.evaluate(() =>
    window.__tabPush({
      state: "listening",
      tabExpanded: true,
      interimText: "A short preview with room above and below.",
    }),
  );
  const row = await page.locator(".tab-control-row").boundingBox();
  const text = await page.locator(".tab-preview").boundingBox();
  const sheet = await page.locator(".tab-sheet").boundingBox();
  expect(text.y - row.y - row.height).toBeGreaterThanOrEqual(9);
  expect(sheet.y + sheet.height - text.y - text.height).toBeGreaterThanOrEqual(
    9,
  );
  await page
    .locator(".screen-tab")
    .screenshot({ path: testInfo.outputPath("preview-spacing.png") });
});

for (const action of ["button", "Escape"]) {
  test(`stop cleanup with ${action} keeps text and uses the cleanup command`, async ({
    page,
  }, testInfo) => {
    await screenTab(page);
    await page.evaluate(() =>
      window.__tabPush({
        state: "cleaning",
        sessionId: 9,
        tabExpanded: true,
        originalText: "Keep the whole recording.",
        interimText: "Keep the whole recording.",
      }),
    );
    const stop = page.getByRole("button", {
      name: "Stop cleanup",
      exact: true,
    });
    await expect(stop).toBeEnabled();
    await expect(
      page.getByRole("button", { name: "Cancel", exact: true }),
    ).toHaveCount(0);
    await expect(page.locator(".tab-shortcut")).toHaveText("Ctrl+Space");
    const buttonBounds = await stop.boundingBox();
    const sheetBounds = await page.locator(".tab-sheet").boundingBox();
    expect(buttonBounds.x + buttonBounds.width).toBeLessThanOrEqual(
      sheetBounds.x + sheetBounds.width,
    );
    if (action === "button") {
      await page
        .locator(".screen-tab")
        .screenshot({ path: testInfo.outputPath("cleanup.png") });
      await stop.click();
    } else {
      await stop.focus();
      await page.keyboard.press("Escape");
    }
    await expect
      .poll(() => page.evaluate(() => window.__tabState().state))
      .toBe("idle");
    const result = await page.evaluate(() => ({
      state: window.__tabState(),
      calls: window.__tabCalls.map((c) => c.command),
    }));
    expect(result.state.outputText).toBe("Keep the whole recording.");
    expect(result.calls).toContain("stop_cleanup");
    expect(result.calls).not.toContain("cancel_dictation");
  });
}

// UI-only command fixture. Native focus, capture and hit testing require the real-app proof.
async function screenTab(page, presentation) {
  await page.addInitScript(() => {
    window.isTauri = true;
    const callbacks = new Map();
    const listeners = new Map();
    let next = 1;
    let releaseStart;
    let releaseCollapse;
    const state = {
      revision: 1,
      sessionId: null,
      tabVisible: true,
      tabExpanded: false,
      tabTemporary: false,
      canCopyLast: false,
      deliveryNotice: null,
      settings: {
        theme: "fog",
        material: true,
        cleanupMode: "clean",
        launchAtLogin: false,
        hotkey: "Ctrl+Space",
        hotkeyMode: "toggle",
        autoPaste: true,
        audioCues: false,
        autoStopSeconds: 0,
        microphone: null,
        asrModel: "moonshine-medium",
        cleanupModel: "local-rules",
        startTabHidden: false,
        cursorIndicator: true,
        meetingSource: "both",
        meetingConsentAcknowledged: true,
        meetingSpeakerLabels: true,
      },
      state: "idle",
      originalText: "",
      outputText: "",
      interimText: "",
      audioLevel: 0,
      error: null,
      models: [
        {
          id: "moonshine-medium",
          kind: "asr",
          ready: true,
          name: "Moonshine Medium",
          description: "UI fixture",
          sizeBytes: 1,
        },
      ],
      devices: [
        { id: "default-mic", name: "Fixture microphone", isDefault: true },
      ],
      meetingRecording: false,
      activeMeetingId: null,
      capabilities: {
        autoPaste: true,
        systemAudio: true,
        systemAudioStatus: "ready",
        globalShortcut: true,
        platform: "UI test only",
      },
      version: "0.10.1",
    };
    // Match Tauri's JSON boundary, including nested Svelte state proxies.
    const clone = (value) =>
      value === undefined ? undefined : JSON.parse(JSON.stringify(value));
    const emit = () => {
      state.revision++;
      callbacks.get(listeners.get("app-state"))?.({
        event: "app-state",
        id: 1,
        payload: clone(state),
      });
    };
    const meeting = {
      id: "test-meeting",
      title: "Meeting",
      createdAt: "2026-09-11T12:00:00Z",
      durationSeconds: 0,
      status: "recording",
      hasAudio: true,
      hasTranscript: false,
      transcript: "",
      notes: "",
      source: "both",
      audioPath: null,
      error: null,
    };
    const systemAudioStatus = () =>
      state.capabilities.systemAudioStatus ??
      (state.capabilities.systemAudio ? "ready" : "unsupported");
    window.__tabCalls = [];
    window.__tabPush = (patch) => {
      Object.assign(state, clone(patch));
      emit();
    };
    window.__tabState = () => clone(state);
    window.__tabReleaseStart = () => releaseStart?.();
    window.__tabReleaseCollapse = () => releaseCollapse?.();
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: (_, id) => callbacks.delete(id),
    };
    window.__TAURI_INTERNALS__ = {
      metadata: {
        currentWindow: { label: "overlay" },
        currentWebview: { label: "overlay", windowLabel: "overlay" },
      },
      transformCallback: (callback) => {
        const id = next++;
        callbacks.set(id, callback);
        return id;
      },
      invoke: async (command, args = {}) => {
        window.__tabCalls.push({ command, args: clone(args) });
        switch (command) {
          case "plugin:event|listen":
            listeners.set(args.event, args.handler);
            return args.handler;
          case "plugin:event|unlisten":
            return null;
          case "get_snapshot":
            return clone(state);
          case "save_settings":
            if (window.__tabFailSettings)
              throw new Error("Could not save settings");
            state.settings = clone(args.settings);
            emit();
            return clone(state);
          case "set_screen_tab": {
            // Mirror Runtime::apply_tab: hover keeps a temporary reveal; a
            // pinned or focus-taking request pins it and a hide dismisses it.
            state.tabTemporary =
              state.tabTemporary && args.visible && !args.focus && !args.pinned;
            state.tabVisible = args.visible;
            state.tabExpanded = args.visible && args.expanded;
            emit();
            const changed = clone(state);
            if (!args.expanded && args.visible && window.__tabDelayCollapse)
              await new Promise((resolve) => {
                releaseCollapse = resolve;
              });
            return changed;
          }
          case "screen_tab_record": {
            state.state = window.__tabStartLoading ? "loading" : "listening";
            state.sessionId = 1;
            state.interimText = "The literal newest words.";
            emit();
            const started = clone(state);
            if (window.__tabDelayStart)
              await new Promise((resolve) => {
                releaseStart = resolve;
              });
            return started;
          }
          case "stop_dictation":
            state.state = "idle";
            state.sessionId = null;
            state.canCopyLast = true;
            state.outputText = "The literal newest words.";
            emit();
            return clone(state);
          case "cancel_dictation":
            state.state = "idle";
            state.sessionId = null;
            state.interimText = "";
            emit();
            return clone(state);
          case "stop_cleanup":
            state.state = "idle";
            state.outputText = state.originalText;
            state.canCopyLast = true;
            emit();
            return clone(state);
          case "copy_text":
          case "copy_last_dictation":
            return null;
          case "system_audio_status":
            return systemAudioStatus();
          case "permission_status":
          case "permission_assistant":
            return { kind: args.kind, required: true,
              granted: systemAudioStatus() === "ready", assistantOpen: false,
              title: "", body: "", staleHint: "" };
          case "screen_tab_meeting": {
            // Mirrors the backend: a refused permission keeps the system side
            // and the recorder reports it as missing.
            meeting.source = state.settings.meetingSource;
            const status = systemAudioStatus();
            state.meetingSystemAudio =
              meeting.source !== "microphone" &&
              (status === "notDetermined" || status === "denied")
                ? "permission"
                : null;
            state.meetingRecording = true;
            state.activeMeetingId = meeting.id;
            emit();
            return clone(meeting);
          }
          case "stop_meeting":
            state.meetingRecording = false;
            emit();
            return clone(meeting);
          case "show_page":
            return null;
          default:
            throw new Error(`Unexpected command ${command}`);
        }
      },
    };
  });
  await page.setViewportSize({ width: 640, height: 450 });
  await page.goto(
    presentation
      ? `/?view=overlay&presentation=${presentation}`
      : "/?view=overlay",
  );
  if (presentation === "menubar") return;
  // Native reveal moves the HWND; this fixture applies the same final translation in CSS.
  await page.addStyleTag({
    content:
      ".screen-tab.native{transform:translateY(calc(-1 * var(--sheet-height)))} .screen-tab.native.expanded{transform:translateY(0)}",
  });
  await expect(page.locator(".tab-lip")).toHaveAttribute(
    "aria-expanded",
    "false",
  );
}

test("idle stays on one row; recording uses one controls row and a full-width preview", async ({
  page,
}, testInfo) => {
  await screenTab(page);
  const collapsed = await page.locator(".tab-lip").boundingBox();
  expect(collapsed).toMatchObject({ x: 275, y: 0, width: 50, height: 26 });
  await expect(page.locator(".tab-lip")).toHaveText("");
  await page.locator(".tab-lip").hover();
  const opened = await page.locator(".tab-lip").boundingBox();
  expect(opened.y).toBe(48);
  const idleControls = page.locator(".tab-control-row");
  const idleRow = await idleControls.boundingBox();
  for (const name of [
    "Record",
    "Copy last",
    "Start meeting",
    "Open app",
    "Hide",
  ]) {
    const bounds = await page
      .getByRole("button", { name, exact: true })
      .boundingBox();
    expect(bounds.y + bounds.height / 2).toBe(idleRow.y + idleRow.height / 2);
    expect(bounds.x).toBeGreaterThanOrEqual(idleRow.x);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(
      idleRow.x + idleRow.width,
    );
  }
  for (const selector of ["[role=status]", ".tab-shortcut"]) {
    const bounds = await idleControls.locator(selector).boundingBox();
    expect(bounds.y + bounds.height / 2).toBe(idleRow.y + idleRow.height / 2);
    expect(
      await idleControls
        .locator(selector)
        .evaluate((element) => element.scrollWidth <= element.clientWidth),
    ).toBe(true);
  }
  await page
    .locator(".screen-tab")
    .screenshot({ path: testInfo.outputPath("idle-browser.png") });
  await page.getByRole("button", { name: "Record", exact: true }).click();
  await page.mouse.move(500, 400);
  await page.waitForTimeout(450);
  await expect(page.locator(".tab-lip")).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  expect((await page.locator(".screen-tab").boundingBox()).height).toBe(104);
  const sheet = await page.locator(".tab-sheet").boundingBox();
  for (const name of ["Stop dictation", "Cancel", "Hide"]) {
    const bounds = await page
      .getByRole("button", { name, exact: true })
      .boundingBox();
    expect(bounds.y).toBeGreaterThanOrEqual(sheet.y);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(
      sheet.y + sheet.height,
    );
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(sheet.x + sheet.width);
  }
  await expect(
    page.getByRole("button", { name: "Copy last", exact: true }),
  ).toHaveCount(0);
  await page.evaluate(() =>
    window.__tabPush({
      settings: {
        ...window.__tabState().settings,
        hotkey: "Ctrl+Shift+Space",
        hotkeyMode: "hold",
      },
    }),
  );
  await expect(page.locator(".tab-shortcut")).toHaveText(
    "Release Ctrl+Shift+Space",
  );
  const controls = await page.locator(".tab-control-row").boundingBox();
  const meter = await page.getByRole("meter").boundingBox();
  const lead = await page.locator(".tab-lead").boundingBox();
  const trail = await page.locator(".tab-trail").boundingBox();
  const preview = await page.getByLabel("Live transcript").boundingBox();
  expect(meter.y + meter.height / 2).toBe(controls.y + controls.height / 2);
  expect(
    Math.abs(meter.x + meter.width / 2 - (lead.x + lead.width + trail.x) / 2),
  ).toBeLessThan(2);
  expect(preview.y).toBeGreaterThanOrEqual(controls.y + controls.height);
  expect(preview.x).toBe(controls.x);
  expect(preview.width).toBe(controls.width);
  expect(
    await page
      .locator(".tab-shortcut")
      .evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
  await page
    .locator(".screen-tab")
    .screenshot({ path: testInfo.outputPath("recording-browser.png") });
});

test("hover corridor reverses a delayed collapse without requesting focus", async ({
  page,
}) => {
  await screenTab(page);
  await page.locator(".tab-lip").hover();
  await expect(
    page.getByRole("button", { name: "Record", exact: true }),
  ).toBeEnabled();
  await page.mouse.move(300, 50);
  await expect(page.locator(".tab-lip")).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  await page.mouse.move(500, 400);
  await page.waitForTimeout(120);
  await page.locator(".tab-lip").hover();
  await page.waitForTimeout(420);
  await expect(page.locator(".tab-lip")).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  const calls = await page.evaluate(() =>
    window.__tabCalls.filter((call) => call.command === "set_screen_tab"),
  );
  expect(calls.every((call) => call.args.focus === false)).toBe(true);
  await page.mouse.move(500, 400);
  await expect(page.locator(".tab-lip")).toHaveAttribute(
    "aria-expanded",
    "false",
  );
});

test("Stop remains available while the start response is pending; late response stays stale", async ({
  page,
}) => {
  await screenTab(page);
  await page.evaluate(() => {
    window.__tabDelayStart = true;
    window.__tabStartLoading = true;
  });
  await page.locator(".tab-lip").hover();
  await page.getByRole("button", { name: "Record", exact: true }).click();
  await page
    .getByRole("button", { name: "Stop dictation", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Copy last", exact: true }),
  ).toBeEnabled();
  await page.evaluate(() => window.__tabReleaseStart());
  await expect(
    page.getByRole("button", { name: "Record", exact: true }),
  ).toBeEnabled();
  expect(await page.evaluate(() => window.__tabState().state)).toBe("idle");
  await page.getByRole("button", { name: "Record", exact: true }).click();
  const cancel = page.getByRole("button", { name: "Cancel", exact: true });
  await expect(cancel).toBeEnabled();
  await expect(cancel).toHaveAttribute(
    "title",
    "Cancel dictation without publishing this recording",
  );
  await cancel.click();
  await page.evaluate(() => window.__tabReleaseStart());
  await expect(
    page.getByRole("button", { name: "Record", exact: true }),
  ).toBeEnabled();
  expect(await page.evaluate(() => window.__tabState().state)).toBe("idle");
  await page.evaluate(() => window.__tabPush({ models: [] }));
  await expect(
    page.getByRole("button", { name: "Record", exact: true }),
  ).toBeDisabled();
});

test("Hide has zero DOM input footprint and does not cancel active capture", async ({
  page,
}) => {
  await screenTab(page);
  await page.locator(".tab-lip").hover();
  await page.getByRole("button", { name: "Record", exact: true }).click();
  await page.getByRole("button", { name: "Hide", exact: true }).click();
  await expect(page.getByRole("group", { name: "Screen tab" })).toBeHidden();
  const result = await page.evaluate(() => {
    const group = document.querySelector(".screen-tab");
    const probe = document.elementFromPoint(300, 12);
    return {
      state: window.__tabState(),
      commands: window.__tabCalls.map((call) => call.command),
      // Boolean, not !== null: elementFromPoint may return null, which the
      // optional chain would leave as undefined — a false positive.
      hit: Boolean(probe?.closest(".screen-tab")),
      probe: probe?.tagName ?? null,
      display: getComputedStyle(group).display,
      pointerEvents: getComputedStyle(group).pointerEvents,
    };
  });
  expect(result.state.tabVisible).toBe(false);
  expect(result.state.state).toBe("listening");
  expect(result.commands).not.toContain("cancel_dictation");
  expect(result.commands).not.toContain("stop_dictation");
  // display:none — visibility:hidden alone is overridden by descendants that
  // set visibility:visible (the brand mark img), which the hit probe found.
  expect(result.display).toBe("none");
  expect(result.pointerEvents).toBe("none");
  expect(result.hit, `probe hit ${result.probe}`).toBe(false);
  await page.evaluate(() => window.__tabPush({ state: "finishing" }));
  await expect(page.getByRole("group", { name: "Screen tab" })).toBeHidden();
  await page.evaluate(() =>
    window.__tabPush({
      state: "listening",
      sessionId: 2,
      tabVisible: true,
      tabExpanded: true,
    }),
  );
  await expect(page.getByRole("group", { name: "Screen tab" })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Stop dictation", exact: true }),
  ).toBeEnabled();
});

test("secondary actions use only their exact commands and a two-sided meeting starts directly", async ({
  page,
}) => {
  await screenTab(page);
  await page.locator(".tab-lip").hover();
  await expect(
    page.getByRole("button", { name: "Copy last", exact: true }),
  ).toBeDisabled();
  await page.evaluate(() =>
    window.__tabPush({
      canCopyLast: true,
      outputText: "A failed later result must not define copy-last.",
    }),
  );
  await page.getByRole("button", { name: "Copy last", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("Last dictation copied.");
  await page.getByRole("button", { name: "Open app", exact: true }).click();
  expect(await page.evaluate(() => window.__tabState().meetingRecording)).toBe(
    false,
  );
  await page
    .getByRole("button", { name: "Start meeting", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Stop meeting", exact: true }),
  ).toBeEnabled();
  const calls = await page.evaluate(() => window.__tabCalls);
  expect(calls).toContainEqual({ command: "copy_last_dictation", args: {} });
  expect(calls).toContainEqual({
    command: "show_page",
    args: { page: "dictation" },
  });
  expect(calls).toContainEqual({ command: "screen_tab_meeting", args: {} });
  expect(calls.some((call) => call.command === "start_meeting")).toBe(false);
});

for (const presentation of [undefined, "menubar"]) {
  for (const source of ["microphone", "system"]) {
    test(`${presentation || "screen tab"} warns before starting a ${source} meeting`, async ({
      page,
    }) => {
      await screenTab(page, presentation);
      await page.evaluate(
        (meetingSource) =>
          window.__tabPush({
            settings: { ...window.__tabState().settings, meetingSource },
            tabExpanded: true,
            tabTemporary: true,
          }),
        source,
      );
      await page
        .getByRole("button", { name: "Start meeting", exact: true })
        .click();
      await expect(page.getByRole("alert")).toContainText(
        source === "microphone"
          ? "Other people on a Zoom, Teams or Meet call won't be in the recording"
          : "Your own voice won't be in the recording",
      );
      expect(
        await page.evaluate(() => window.__tabState().meetingRecording),
      ).toBe(false);
      const sheet = await page.locator(".tab-sheet").boundingBox();
      const warning = await page.getByRole("alert").boundingBox();
      expect(warning.y).toBeGreaterThanOrEqual(sheet.y);
      expect(warning.y + warning.height).toBeLessThanOrEqual(
        sheet.y + sheet.height - 4,
      );
      await page
        .getByRole("button", { name: "Start anyway", exact: true })
        .click();
      await expect(
        page.getByRole("button", { name: "Stop meeting", exact: true }),
      ).toBeEnabled();
      expect(
        await page.evaluate(() => window.__tabState().settings.meetingSource),
      ).toBe(source);
      expect(
        await page.evaluate(
          () =>
            window.__tabCalls.filter((c) => c.command === "screen_tab_meeting")
              .length,
        ),
      ).toBe(1);
    });
  }
}

test("Record both sides saves before starting; a failed save never starts", async ({
  page,
}) => {
  await screenTab(page);
  await page.evaluate(() =>
    window.__tabPush({
      settings: {
        ...window.__tabState().settings,
        meetingSource: "microphone",
      },
      tabExpanded: true,
    }),
  );
  await page
    .getByRole("button", { name: "Start meeting", exact: true })
    .click();
  await page.evaluate(() => {
    window.__tabFailSettings = true;
  });
  await page
    .getByRole("button", { name: "Record both sides", exact: true })
    .click();
  expect(await page.evaluate(() => window.__tabState().meetingRecording)).toBe(
    false,
  );
  expect(
    await page.evaluate(() =>
      window.__tabCalls.some((c) => c.command === "screen_tab_meeting"),
    ),
  ).toBe(false);
  await page.evaluate(() => {
    window.__tabFailSettings = false;
  });
  await page
    .getByRole("button", { name: "Start meeting", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Record both sides", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Stop meeting", exact: true }),
  ).toBeEnabled();
  const calls = await page.evaluate(() =>
    window.__tabCalls.filter((c) =>
      ["save_settings", "screen_tab_meeting"].includes(c.command),
    ),
  );
  expect(calls.map((c) => c.command)).toEqual([
    "save_settings",
    "save_settings",
    "screen_tab_meeting",
  ]);
  expect(calls[1].args.settings.meetingSource).toBe("both");
});

test("Cancel and Escape dismiss the meeting warning without capture", async ({
  page,
}) => {
  await screenTab(page);
  await page.evaluate(() =>
    window.__tabPush({
      settings: { ...window.__tabState().settings, meetingSource: "system" },
      tabExpanded: true,
    }),
  );
  for (const action of ["cancel", "escape"]) {
    await page
      .getByRole("button", { name: "Start meeting", exact: true })
      .click();
    if (action === "cancel")
      await page.getByRole("button", { name: "Cancel meeting start" }).click();
    else {
      await page
        .getByRole("button", { name: "Start anyway", exact: true })
        .focus();
      await page.keyboard.press("Escape");
    }
    await expect(page.getByRole("alert")).toHaveCount(0);
    expect(
      await page.evaluate(() => window.__tabState().meetingRecording),
    ).toBe(false);
  }
});

test("keyboard focus keeps controls open; Escape collapses without canceling", async ({
  page,
}) => {
  await screenTab(page);
  await page.keyboard.press("Tab");
  await expect(page.locator(".tab-lip")).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(
    page.getByRole("button", { name: "Record", exact: true }),
  ).toBeFocused();
  await page.mouse.move(500, 400);
  await page.waitForTimeout(450);
  await expect(page.locator(".tab-lip")).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  await page.keyboard.press("Enter");
  await expect(
    page.getByRole("button", { name: "Stop dictation", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.locator(".tab-lip")).toHaveAttribute(
    "aria-expanded",
    "false",
  );
  expect(await page.evaluate(() => window.__tabState().state)).toBe(
    "listening",
  );
  expect(
    await page.evaluate(() =>
      window.__tabCalls.some((call) => call.command === "cancel_dictation"),
    ),
  ).toBe(false);
});

test("the bounded live suffix and controls survive reduced motion and theme changes", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await screenTab(page);
  // The preview mounts on the idle → listening transition; the overflow
  // observer must attach to elements that did not exist at init.
  await expect(page.getByLabel("Live transcript")).toHaveCount(0);
  await page.locator(".tab-lip").hover();
  await page.evaluate(() =>
    window.__tabPush({ state: "listening", interimText: "Short sentence." }),
  );
  const shortPreview = page.getByLabel("Live transcript");
  await expect(shortPreview).toHaveText("Short sentence.");
  await page.waitForTimeout(120);
  await expect(shortPreview).not.toHaveClass(/overflowing/);
  const text = "Earlier literal words ".repeat(80) + "Newest words 🪶";
  await page.evaluate(
    (interimText) => window.__tabPush({ state: "listening", interimText }),
    text,
  );
  const preview = await page.getByLabel("Live transcript").textContent();
  expect(preview.trim().endsWith("Newest words 🪶")).toBe(true);
  expect(preview.includes("…")).toBe(false);
  expect(Array.from(preview.trim()).length).toBe(280);
  await expect(page.getByLabel("Live transcript")).toHaveClass(/overflowing/);
  const suffixBounds = await page
    .getByLabel("Live transcript")
    .evaluate((element) => {
      const text = element.querySelector("span").firstChild;
      const range = document.createRange();
      range.setStart(text, text.textContent.length - 2);
      range.setEnd(text, text.textContent.length);
      return {
        lastLine: range.getBoundingClientRect().bottom,
        viewport: element.getBoundingClientRect().bottom,
        suffixRight: range.getBoundingClientRect().right,
        viewportRight: element.getBoundingClientRect().right,
        height: element.getBoundingClientRect().height,
      };
    });
  expect(suffixBounds.lastLine).toBeLessThanOrEqual(suffixBounds.viewport + 1);
  expect(suffixBounds.lastLine).toBeGreaterThan(suffixBounds.viewport - 20);
  expect(suffixBounds.suffixRight).toBeLessThanOrEqual(
    suffixBounds.viewportRight + 1,
  );
  expect(suffixBounds.suffixRight).toBeGreaterThan(
    suffixBounds.viewportRight - 3,
  );
  expect(suffixBounds.height).toBe(18);
  await page.evaluate(() =>
    window.__tabPush({
      settings: {
        ...window.__tabState().settings,
        theme: "slate",
        material: false,
      },
    }),
  );
  await expect(page.locator(".studio")).toHaveAttribute("data-theme", "slate");
  await expect(page.locator(".studio")).toHaveAttribute("data-material", "off");
  const sheet = await page.locator(".tab-sheet").boundingBox();
  const button = await page
    .getByRole("button", { name: "Stop dictation", exact: true })
    .boundingBox();
  expect(button.y + button.height).toBeLessThan(sheet.y + sheet.height);
});

test("compact errors open their repair page and leave Stop available", async ({
  page,
}) => {
  await screenTab(page);
  await page.locator(".tab-lip").hover();
  await page.evaluate(() =>
    window.__tabPush({
      state: "listening",
      error: "Microphone device 0x80004002 at C:/private/test.dll",
    }),
  );
  await expect(page.getByRole("alert")).toContainText(
    "The microphone is unavailable",
  );
  await expect(page.getByRole("alert")).not.toContainText("0x80004002");
  const alert = await page.getByRole("alert").boundingBox();
  const stop = await page
    .getByRole("button", { name: "Stop dictation", exact: true })
    .boundingBox();
  expect(alert.y).toBeGreaterThanOrEqual(stop.y + stop.height);
  expect(alert.y + alert.height).toBeLessThanOrEqual(78);
  await page.getByRole("button", { name: "Open Setup", exact: true }).click();
  expect(await page.evaluate(() => window.__tabCalls)).toContainEqual({
    command: "show_page",
    args: { page: "setup" },
  });
  await page
    .getByRole("button", { name: "Stop dictation", exact: true })
    .click();
  expect(await page.evaluate(() => window.__tabState().state)).toBe("idle");
});

test("paper variants retain the same geometry", async ({ page }, testInfo) => {
  await screenTab(page);
  await page.locator(".tab-lip").hover();
  for (const theme of ["fog", "slate"])
    for (const material of [true, false]) {
      await page.evaluate(
        ({ theme, material }) =>
          window.__tabPush({
            settings: { ...window.__tabState().settings, theme, material },
          }),
        { theme, material },
      );
      await expect(page.locator(".studio")).toHaveAttribute(
        "data-theme",
        theme,
      );
      await page.locator(".screen-tab").screenshot({
        path: testInfo.outputPath(
          `${theme}-${material ? "material" : "flat"}-browser.png`,
        ),
      });
      const geometry = await page.locator(".tab-sheet").boundingBox();
      expect(geometry.width).toBe(600);
      expect(geometry.height).toBe(48);
      const lipColor = await page
        .locator(".tab-lip")
        .evaluate((element) => getComputedStyle(element).backgroundColor);
      const sheetColor = await page
        .locator(".tab-sheet")
        .evaluate((element) => getComputedStyle(element).backgroundColor);
      expect(lipColor).not.toBe(sheetColor);
      await expect(
        page.locator(`.tab-lip .${theme === "fog" ? "white" : "pine"}`),
      ).toBeVisible();
      await expect(
        page.locator(`.tab-lip .${theme === "fog" ? "pine" : "white"}`),
      ).toBeHidden();
    }
});

test("a temporary reveal offers Keep to pin and Hide to dismiss", async ({
  page,
}) => {
  await screenTab(page);
  const tabCalls = () =>
    page.evaluate(() =>
      window.__tabCalls.filter((call) => call.command === "set_screen_tab"),
    );
  await page.evaluate(() =>
    window.__tabPush({
      tabVisible: true,
      tabExpanded: true,
      tabTemporary: true,
    }),
  );
  const keep = page.getByRole("button", { name: "Keep", exact: true });
  await expect(keep).toBeVisible();
  // Hovering the revealed sheet must not pin it.
  await page.mouse.move(300, 24);
  expect((await tabCalls()).at(-1)).toMatchObject({
    args: { visible: true, expanded: true, focus: false },
  });
  expect(await page.evaluate(() => window.__tabState().tabTemporary)).toBe(
    true,
  );
  await keep.click();
  expect((await tabCalls()).at(-1)).toMatchObject({
    args: { visible: true, expanded: true, focus: false, pinned: true },
  });
  expect(await page.evaluate(() => window.__tabState().tabTemporary)).toBe(
    false,
  );
  await expect(keep).toHaveCount(0);
  // Recording keeps the same pin beside Cancel and Hide.
  await page.evaluate(() =>
    window.__tabPush({
      state: "listening",
      sessionId: 3,
      tabVisible: true,
      tabExpanded: true,
      tabTemporary: true,
    }),
  );
  await expect(
    page.getByRole("button", { name: "Keep visible", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Hide", exact: true }).click();
  expect((await tabCalls()).at(-1)).toMatchObject({
    args: { visible: false },
  });
  expect(await page.evaluate(() => window.__tabState().tabTemporary)).toBe(
    false,
  );
  expect(await page.evaluate(() => window.__tabState().state)).toBe(
    "listening",
  );
  await expect(page.getByRole("group", { name: "Screen tab" })).toBeHidden();
});

test("a capture event during collapse is not undone by the old geometry response", async ({
  page,
}) => {
  await screenTab(page);
  await page.locator(".tab-lip").hover();
  await page.evaluate(() => {
    window.__tabDelayCollapse = true;
  });
  await page.mouse.move(500, 400);
  await expect(page.locator(".tab-lip")).toHaveAttribute(
    "aria-expanded",
    "false",
  );
  await page.evaluate(() =>
    window.__tabPush({ state: "listening", sessionId: 2, tabExpanded: true }),
  );
  await page.evaluate(() => window.__tabReleaseCollapse());
  await expect(page.locator(".tab-lip")).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  await page
    .getByRole("button", { name: "Stop dictation", exact: true })
    .click();
  expect(await page.evaluate(() => window.__tabState().state)).toBe("idle");
});

test("a two-source meeting shows separate microphone and system meters", async ({
  page,
}) => {
  await screenTab(page);
  await page.locator(".tab-lip").hover();
  await page
    .getByRole("button", { name: "Start meeting", exact: true })
    .click();
  // The mixed meter stays until per-source levels arrive.
  await expect(page.locator(".tab-stems")).toHaveCount(0);
  await expect(page.getByRole("meter")).toHaveCount(1);
  await page.evaluate(() =>
    window.__tabPush({ meetingLevels: { microphone: 0.2, system: 0.05 } }),
  );
  await expect(page.locator(".tab-stem")).toHaveCount(2);
  await expect(page.getByTitle("Your microphone")).toBeVisible();
  await expect(page.getByTitle("Other speakers (system audio)")).toBeVisible();
});

test("the menu bar sheet has no lip and stays open when the pointer leaves", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await screenTab(page, "menubar");
  await expect(
    page.getByRole("group", { name: "Dictation controls" }),
  ).toBeVisible();
  await expect(page.locator(".tab-lip")).toHaveCount(0);
  const idle = await page.locator(".tab-sheet").boundingBox();
  expect(idle).toMatchObject({ x: 0, y: 0, width: 600, height: 48 });
  await page.mouse.move(12, 220);
  await page.waitForTimeout(500);
  await expect(page.locator(".tab-sheet")).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        window.__tabCalls.filter((call) => call.command === "set_screen_tab")
          .length,
    ),
  ).toBe(0);
  await page.evaluate(() =>
    window.__tabPush({
      state: "listening",
      interimText: "The newest words stay on one line.",
    }),
  );
  const recording = await page.locator(".tab-sheet").boundingBox();
  expect(recording.height).toBe(78);
  await page
    .getByRole("button", { name: "Stop dictation", exact: true })
    .focus();
  await page.keyboard.press("Escape");
  const hidden = await page.evaluate(() =>
    window.__tabCalls.filter((call) => call.command === "set_screen_tab"),
  );
  expect(hidden.at(-1).args).toMatchObject({ visible: false, expanded: false });
});

for (const theme of ["fog", "slate"]) {
  test(`a quick start without system audio records the microphone and shows the fix (${theme})`, async ({
    page,
  }) => {
    await screenTab(page);
    await page.evaluate(
      (theme) =>
        window.__tabPush({
          settings: { ...window.__tabState().settings, theme },
          capabilities: {
            ...window.__tabState().capabilities,
            systemAudio: false,
            systemAudioStatus: "denied",
          },
          tabExpanded: true,
        }),
      theme,
    );
    await page
      .getByRole("button", { name: "Start meeting", exact: true })
      .click();
    // Nothing waits on a fix: the meeting is already recording.
    await expect(
      page.getByRole("button", { name: "Stop meeting", exact: true }),
    ).toBeEnabled();
    const alert = page.getByRole("alert");
    await expect(alert).toContainText(
      "System audio is off. Other people on the call aren't being recorded.",
    );
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({
      path: `../artifacts/mac-audio-review/tab-recording-system-off-${theme}.png`,
      clip: { x: 0, y: 0, width: 640, height: 110 },
    });
    await alert
      .getByRole("button", { name: "Allow system audio", exact: true })
      .click();
    const calls = await page.evaluate(() => window.__tabCalls);
    expect(calls).toContainEqual({
      command: "permission_assistant",
      args: { kind: "system_audio", action: "open" },
    });
    expect(
      calls.filter((call) => call.command === "screen_tab_meeting"),
    ).toHaveLength(1);
  });
}

test("a microphone-only quick start without permission offers the fix, not both sides", async ({
  page,
}) => {
  await screenTab(page);
  await page.evaluate(() =>
    window.__tabPush({
      settings: { ...window.__tabState().settings, meetingSource: "microphone" },
      capabilities: {
        ...window.__tabState().capabilities,
        systemAudio: false,
        systemAudioStatus: "notDetermined",
      },
      tabExpanded: true,
    }),
  );
  await page.getByRole("button", { name: "Start meeting", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Other people on a Zoom, Teams or Meet call won't be in the recording",
  );
  await expect(
    page.getByRole("button", { name: "Record both sides", exact: true }),
  ).toHaveCount(0);
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({
    path: "../artifacts/mac-audio-review/tab-microphone-only-fix-fog.png",
    clip: { x: 0, y: 0, width: 640, height: 90 },
  });
  await page
    .getByRole("button", { name: "Fix system audio", exact: true })
    .click();
  const calls = await page.evaluate(() => window.__tabCalls);
  expect(calls).toContainEqual({
    command: "permission_assistant",
    args: { kind: "system_audio", action: "open" },
  });
  expect(calls.some((call) => call.command === "screen_tab_meeting")).toBe(false);
  expect(await page.evaluate(() => window.__tabState().meetingRecording)).toBe(false);
});
