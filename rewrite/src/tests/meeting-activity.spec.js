import { expect, test } from "@playwright/test";
import {
  installDesktopFixture,
  structuredMeetingTranscript,
} from "./desktop-fixture.js";

async function openMeetings(page, theme = "fog") {
  await installDesktopFixture(page);
  await page.goto(`/?page=meetings&theme=${theme}`);
  await page.evaluate((theme) => {
    const snapshot = window.__testSnapshot();
    window.__testPushSnapshot({ settings: { ...snapshot.settings, theme } });
  }, theme);
  await expect(page.locator(".studio")).toHaveAttribute("data-theme", theme);
  await expect(
    page.getByRole("button", { name: /Design review/ }),
  ).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

async function updateMeeting(page, patch) {
  await page.evaluate((patch) => window.__testUpdateMeeting(patch), patch);
}

for (const theme of ["fog", "slate"]) {
  test(`${theme}: structured transcripts keep speaker alignment and timecodes through edits`, async ({
    page,
  }, testInfo) => {
    await installDesktopFixture(page, {
      meeting: {
        transcript: structuredMeetingTranscript,
        source: "both",
        transcriptionModel: "moonshine-medium",
      },
    });
    await page.goto(`/?page=meetings&theme=${theme}`);
    await page.getByRole("button", { name: /Design review/ }).click();
    await expect(page.locator("#meeting-transcript")).toHaveCount(0);
    const blocks = page.locator(".transcript-block");
    await expect(blocks).toHaveCount(3);
    await expect(blocks.locator("time")).toHaveText([
      "0:12",
      "0:31",
      "1:02:03",
    ]);
    await expect(blocks.nth(0)).toHaveClass(/transcript-you/);
    await expect(blocks.nth(1)).toHaveClass(/transcript-other/);
    await expect(blocks.nth(2)).toHaveClass(/transcript-you/);
    const you = await blocks.nth(0).boundingBox();
    const other = await blocks.nth(1).boundingBox();
    // You sit on the right, other speakers on the left.
    expect(you.x).toBeGreaterThan(other.x);
    await page
      .getByRole("textbox", { name: "Other speakers", exact: true })
      .fill("I will check the release notes today.");
    await page
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect(
      page.getByRole("button", { name: "Save changes", exact: true }),
    ).toBeDisabled();
    expect(
      await page.evaluate(
        () =>
          window.__testCalls.find((call) => call.command === "save_meeting")
            ?.args.transcript,
      ),
    ).toBe(
      structuredMeetingTranscript.replace(
        "release notes.",
        "release notes today.",
      ),
    );
    await expect(blocks.locator("time")).toHaveText([
      "0:12",
      "0:31",
      "1:02:03",
    ]);
    await page.screenshot({
      path: testInfo.outputPath(`structured-transcript-${theme}.png`),
    });
  });
}

for (const config of [
  { label: "disconnected", connected: false, speechModel: "mock/audio" },
  {
    label: "connected without a speech model",
    connected: true,
    speechModel: "",
  },
]) {
  test(`meeting remote speech is absent when ${config.label}`, async ({
    page,
  }) => {
    await installDesktopFixture(page, {
      aiSettings: { ...config, backend: "openrouter", model: "mock/text" },
    });
    await page.goto("/?page=meetings");
    await page.getByRole("button", { name: /Design review/ }).click();
    await page
      .getByRole("button", { name: "Transcribe again", exact: true })
      .click();
    await expect(
      page.locator('#transcription-model option[value="openrouter"]'),
    ).toHaveCount(0);
    await expect(page.getByLabel("Speech model", { exact: true })).toHaveValue(
      "moonshine-medium",
    );
    await expect(
      page.locator(".transcribe-again .remote-model-badge"),
    ).toHaveCount(0);
  });
}

test("losing the OpenRouter connection disables the selected route without a local fallback", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    aiSettings: { connected: true, speechModel: "mock/audio" },
  });
  await page.goto("/?page=meetings");
  await page.getByRole("button", { name: /Design review/ }).click();
  await page
    .getByRole("button", { name: "Transcribe again", exact: true })
    .click();
  await page
    .getByLabel("Speech model", { exact: true })
    .selectOption("openrouter");
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.evaluate(() => window.__testPatchAiSettings({ connected: false }));
  await page
    .getByRole("button", { name: "Transcribe again", exact: true })
    .click();
  await expect(
    page.locator('#transcription-model option[value="openrouter"]'),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Start", exact: true }),
  ).toBeDisabled();
  expect(
    await page.evaluate(() =>
      window.__testCalls.filter(
        (call) => call.command === "transcribe_meeting",
      ),
    ),
  ).toEqual([]);
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await page
    .getByRole("button", { name: "Transcribe again", exact: true })
    .click();
  await expect(page.getByLabel("Speech model", { exact: true })).toHaveValue(
    "",
  );
  await expect(
    page.getByRole("button", { name: "Start", exact: true }),
  ).toBeDisabled();
  await page
    .getByLabel("Speech model", { exact: true })
    .selectOption("moonshine-medium");
  await expect(
    page.getByRole("button", { name: "Start", exact: true }),
  ).toBeEnabled();
});

test("meeting remote speech works with local text settings and retains the model that ran", async ({
  page,
}, testInfo) => {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await installDesktopFixture(page, {
    aiSettings: {
      backend: "local",
      connected: true,
      model: "",
      speechModel: "mock/audio",
    },
  });
  await page.goto("/?page=meetings");
  await page.getByRole("button", { name: /Design review/ }).click();
  await page
    .getByRole("button", { name: "Transcribe again", exact: true })
    .click();
  const select = page.getByLabel("Speech model", { exact: true });
  await expect(
    page.locator('#transcription-model option[value="openrouter"]'),
  ).toHaveText("OpenRouter · Mock speech model");
  await expect(select).toHaveValue("moonshine-medium");
  await select.selectOption("openrouter");
  await select.focus();
  await page.keyboard.press("Tab");
  const badge = page
    .locator(".transcribe-again")
    .getByRole("button", { name: "Remote model privacy", exact: true });
  await expect(badge).toBeFocused();
  const tooltip = page.getByRole("tooltip");
  const warning =
    "Not a local model. Audio or text is sent to OpenRouter and may not be private.";
  await expect(tooltip).toHaveText(warning);
  await expect(badge).toHaveAccessibleDescription(warning);
  await page.screenshot({
    path: testInfo.outputPath("remote-speech-tooltip.png"),
  });
  await page.keyboard.press("Escape");
  await expect(tooltip).toHaveCount(0);
  await badge.hover();
  await expect(tooltip).toHaveText(warning);
  await page.getByRole("button", { name: "Start", exact: true }).click();
  await expect(page.locator(".meeting-heading").getByRole("status")).toHaveText(
    "Transcribing",
  );
  expect(
    await page.evaluate(() =>
      window.__testCalls.filter(
        (call) => call.command === "transcribe_meeting",
      ),
    ),
  ).toEqual([
    {
      command: "transcribe_meeting",
      args: { id: "meeting-one", model: "openrouter" },
    },
  ]);
  await page.evaluate(() => {
    window.__testPatchAiSettings({ speechModel: "mock/changed" });
    window.__testCompleteTranscription();
  });
  await expect(page.locator(".transcribe-model")).toContainText(
    "Last transcribed with OpenRouter · Mock speech model",
  );
  await page
    .locator(".transcribe-model")
    .getByRole("button", { name: "Remote model privacy" })
    .focus();
  await expect(tooltip).toHaveText(warning);
  expect(
    await page.evaluate(() => window.__testSnapshot().settings.asrModel),
  ).toBe("moonshine-medium");
  expect(errors).toEqual([]);
});

async function expectActive(badge, label, reduced = false) {
  await expect(badge).toHaveText(label);
  await expect(badge).toHaveClass(/\bactive\b/);
  await expect(badge).toHaveAttribute("title", label);
  await expect(badge.locator("svg")).toHaveAttribute("aria-hidden", "true");
  const pen = badge.locator(".writing-pen");
  await expect(pen).toBeVisible();
  await expect
    .poll(() =>
      pen.evaluate((element) => {
        const style = getComputedStyle(element);
        return {
          animated: style.animationName !== "none",
          repeating: style.animationIterationCount === "infinite",
        };
      }),
    )
    .toEqual({ animated: !reduced, repeating: !reduced });
  await expect(badge).not.toContainText("%");
}

for (const theme of ["fog", "slate"]) {
  test(`${theme}: transcription and notes animate in the list and heading`, async ({
    page,
  }, testInfo) => {
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await openMeetings(page, theme);
    // A completed sibling must never inherit another meeting's activity.
    await page.evaluate(() => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      window.__TAURI_INTERNALS__.invoke = async (command, args) => {
        const result = await invoke(command, args);
        return command === "list_meetings"
          ? [
              ...result,
              {
                ...result[0],
                id: "finished",
                title: "Finished planning",
                status: "processed",
              },
            ]
          : result;
      };
    });
    await updateMeeting(page, { status: "transcribing" });
    const row = page.getByRole("button", { name: /Design review/ });
    await expectActive(row.locator(".meeting-activity"), "Transcribing");
    await expect(row).toHaveAccessibleName(/Design review.*Transcribing/);
    const finished = page.getByRole("button", { name: /Finished planning/ });
    await expect(finished.locator(".meeting-activity")).toHaveText(
      "Notes ready",
    );
    await expect(finished.locator(".writing-mark")).toHaveCount(0);
    await page.screenshot({
      path: testInfo.outputPath(`list-transcribing-${theme}.png`),
    });

    await row.click();
    const heading = page.locator(".meeting-heading");
    const badge = heading.getByRole("status");
    await expectActive(badge, "Transcribing");
    await expect(badge).toHaveAttribute("aria-live", "polite");
    await expect(badge).toHaveAttribute("aria-atomic", "true");
    const pen = badge.locator(".writing-pen");
    const initialTransform = await pen.evaluate(
      (element) => getComputedStyle(element).transform,
    );
    const labelBox = await badge.boundingBox();
    await expect
      .poll(() =>
        pen.evaluate((element) => getComputedStyle(element).transform),
      )
      .not.toBe(initialTransform);
    expect(await badge.boundingBox()).toEqual(labelBox);
    await page.screenshot({
      path: testInfo.outputPath(`heading-transcribing-${theme}.png`),
    });

    await page
      .getByRole("tab", { name: "Processed notes", exact: true })
      .click();
    await updateMeeting(page, { status: "processingNotes" });
    await expectActive(badge, "Writing notes");
    await expect(
      page.getByRole("tab", { name: "Processed notes", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(page.locator("#meeting-notes")).toBeDisabled();
    await expect(
      page.getByRole("button", { name: "Cancel processing", exact: true }),
    ).toBeEnabled();
    await page.screenshot({
      path: testInfo.outputPath(`heading-notes-${theme}.png`),
    });
    await page
      .getByRole("button", { name: "All meetings", exact: true })
      .click();
    await expectActive(row.locator(".meeting-activity"), "Writing notes");
    await page.screenshot({
      path: testInfo.outputPath(`list-notes-${theme}.png`),
    });
    expect(errors).toEqual([]);
  });
}

test("processing aliases retain their full labels on both surfaces", async ({
  page,
}) => {
  await openMeetings(page);
  const aliases = [
    ["processing", "Processing"],
    ["generatingTitle", "Naming meeting"],
    ["importing", "Importing"],
    ["generating_notes", "Writing notes"],
    ["generating-notes", "Writing notes"],
  ];
  for (const [status, label] of aliases) {
    await test.step(status, async () => {
      await updateMeeting(page, { status });
      const row = page.getByRole("button", { name: /Design review/ });
      await expectActive(row.locator(".meeting-activity"), label);
      await row.click();
      await expectActive(
        page.locator(".meeting-heading").getByRole("status"),
        label,
      );
      await page
        .getByRole("button", { name: "All meetings", exact: true })
        .click();
    });
  }
});

test("completion, errors and cancellation remove activity; recording stays red", async ({
  page,
}) => {
  await openMeetings(page);
  await updateMeeting(page, { status: "transcribing" });
  await expectActive(
    page.locator(".meeting-item .meeting-activity"),
    "Transcribing",
  );
  const statuses = [
    ["transcribed", "Transcribed"],
    ["processed", "Notes ready"],
    ["ready", "Ready"],
    ["complete", "Ready"],
    ["completed", "Ready"],
    ["error", "Needs attention"],
    ["failed", "Needs attention"],
    ["interrupted", "Interrupted"],
    ["cancelled", "Cancelled"],
    ["canceled", "Cancelled"],
    ["recorded", "Recorded"],
    ["imported", "Imported"],
    ["recording", "Recording"],
  ];
  for (const [status, label] of statuses) {
    await test.step(status, async () => {
      await updateMeeting(page, { status });
      const row = page.getByRole("button", { name: /Design review/ });
      const listBadge = row.locator(".meeting-activity");
      await expect(listBadge).toHaveText(label);
      await expect(listBadge).not.toHaveClass(/\bactive\b/);
      await expect(listBadge.locator("svg")).toHaveCount(0);
      await row.click();
      const headingBadge = page.locator(".meeting-heading").getByRole("status");
      await expect(headingBadge).toHaveText(label);
      await expect(headingBadge.locator("svg")).toHaveCount(0);
      expect(
        await headingBadge.evaluate(
          (element) => element.getAnimations({ subtree: true }).length,
        ),
      ).toBe(0);
      if (status === "recording") {
        await expect(headingBadge).toHaveClass(/\brecording\b/);
        expect(
          await headingBadge.evaluate((element) => {
            const probe = document.createElement("span");
            probe.style.color = "var(--danger)";
            element.append(probe);
            const matches =
              getComputedStyle(element).color === getComputedStyle(probe).color;
            probe.remove();
            return matches;
          }),
        ).toBe(true);
      }
      await page
        .getByRole("button", { name: "All meetings", exact: true })
        .click();
    });
  }
});

test("reduced motion keeps a clear static badge and long titles fit a narrow list", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 720, height: 860 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openMeetings(page, "slate");
  await updateMeeting(page, {
    title: "Design review with a longer meeting title to check wrapping",
    status: "processingNotes",
  });
  const row = page.getByRole("button", { name: /Design review/ });
  const badge = row.locator(".meeting-activity");
  await expectActive(badge, "Writing notes", true);
  const rowBox = await row.boundingBox();
  const badgeBox = await badge.boundingBox();
  const titleBox = await row.locator("strong").boundingBox();
  expect(badgeBox.x).toBeGreaterThanOrEqual(rowBox.x);
  expect(badgeBox.x + badgeBox.width).toBeLessThanOrEqual(
    rowBox.x + rowBox.width,
  );
  expect(
    badgeBox.y >= titleBox.y + titleBox.height ||
      badgeBox.x >= titleBox.x + titleBox.width,
  ).toBe(true);
  await page.screenshot({
    path: testInfo.outputPath("list-reduced-motion-narrow.png"),
  });
  await row.click();
  const headingBadge = page.locator(".meeting-heading").getByRole("status");
  await expectActive(headingBadge, "Writing notes", true);
  expect(
    await headingBadge
      .locator(".writing-pen")
      .evaluate((element) => getComputedStyle(element).transform),
  ).toBe("none");
  expect(
    await headingBadge.evaluate(
      (element) => element.getAnimations({ subtree: true }).length,
    ),
  ).toBe(0);
  await page.screenshot({
    path: testInfo.outputPath("heading-reduced-motion-narrow.png"),
  });
});

test("activity updates preserve drafts, tabs and the cancel command", async ({
  page,
}) => {
  await openMeetings(page);
  await page.getByRole("button", { name: /Design review/ }).click();
  await page
    .locator("#meeting-transcript")
    .fill("My unsaved transcript correction.");
  await page.getByRole("tab", { name: "Processed notes", exact: true }).click();
  await updateMeeting(page, {
    status: "processingNotes",
    transcript: "Background transcript.",
  });
  await expectActive(
    page.locator(".meeting-heading").getByRole("status"),
    "Writing notes",
  );
  await expect(
    page.getByRole("tab", { name: "Processed notes", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByText("Unsaved changes", { exact: true }),
  ).toBeVisible();
  await updateMeeting(page, { status: "processed" });
  await expect(page.locator(".meeting-heading").getByRole("status")).toHaveText(
    "Notes ready",
  );
  await page.getByRole("tab", { name: "Full transcript", exact: true }).click();
  await expect(page.locator("#meeting-transcript")).toHaveValue(
    "My unsaved transcript correction.",
  );
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await page.getByRole("tab", { name: "Full transcript", exact: true }).click();
  await page
    .getByRole("button", { name: "Transcribe again", exact: true })
    .click();
  await page
    .getByLabel("Speech model", { exact: true })
    .selectOption("mock-whisper-small-cpu");
  await page.getByRole("button", { name: "Start", exact: true }).click();
  await expectActive(
    page.locator(".meeting-heading").getByRole("status"),
    "Transcribing",
  );
  await page
    .getByRole("button", { name: "Cancel processing", exact: true })
    .click();
  await expect(page.locator(".meeting-heading").getByRole("status")).toHaveText(
    "Cancelled",
  );
  await expect(page.locator(".meeting-activity.active")).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        window.__testCalls.find((call) => call.command === "cancel_meeting")
          ?.args,
    ),
  ).toEqual({ id: "meeting-one" });
});
