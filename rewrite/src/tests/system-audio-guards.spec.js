import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture, macProfileFixture } from "./desktop-fixture.js";

// Review captures for the system audio guards, 1220x860, both themes.
const directory = new URL("../../../artifacts/mac-audio-review/", import.meta.url);
const micOnly =
  "Only your microphone will be recorded. Other people on a Zoom, Teams or Meet call won't be in the recording.";
const calls = (page, command) =>
  page.evaluate(
    (command) => window.__testCalls.filter((call) => call.command === command),
    command,
  );
async function capture(page, name) {
  await mkdir(directory, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  // Let the paper dialog finish its reveal before capturing it.
  if (await page.locator("dialog.confirmation").count()) {
    await expect(page.locator("dialog.confirmation.is-open")).toBeVisible();
    await page.waitForTimeout(400);
  }
  await page.screenshot({ path: fileURLToPath(new URL(`${name}.png`, directory)) });
}
const mac = (status, extra = {}) => {
  const fixture = macProfileFixture(16);
  return {
    ...fixture,
    ...extra,
    settings: { ...fixture.settings, setupProfile: "balanced", ...extra.settings },
    capabilities: { ...fixture.capabilities, systemAudio: status === "ready", systemAudioStatus: status },
  };
};
async function startPage(page, options, theme = "fog") {
  await installDesktopFixture(page, options);
  await page.goto(`/?page=meetings&theme=${theme}`);
  await page.getByRole("button", { name: "Start a meeting", exact: true }).click();
}

for (const theme of ["fog", "slate"]) {
  test(`Mac without permission: the dialog offers the fix before anything records (${theme})`, async ({ page }) => {
    await startPage(page, mac("notDetermined"), theme);
    const note = page.getByRole("status", { name: "System audio", exact: true });
    await expect(note).toContainText("System audio: Not allowed yet.");
    await expect(page.getByRole("button", { name: "Microphone and system audio" })).toBeEnabled();
    await capture(page, `meetings-start-needs-permission-${theme}`);
    await page.getByRole("button", { name: "Start recording" }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("heading")).toHaveText("System audio is off");
    await expect(dialog).toContainText(micOnly);
    await expect(dialog).toContainText("SonoJot needs your permission to record system audio.");
    await expect(dialog.getByRole("button")).toHaveText(["Cancel", "Record microphone only", "Fix system audio"]);
    await capture(page, `dialog-system-off-${theme}`);
    await dialog.getByRole("button", { name: "Fix system audio" }).click();
    await expect(dialog).toHaveCount(0);
    expect((await calls(page, "permission_assistant"))[0].args).toEqual({ kind: "system_audio", action: "open" });
    expect(await calls(page, "start_meeting")).toHaveLength(0);
  });
}

test("Record microphone only keeps the system side and the recording names what is missing", async ({ page }) => {
  await startPage(page, mac("denied"));
  await expect(page.getByRole("status", { name: "System audio", exact: true })).toContainText(
    "turned off in System Settings",
  );
  await page.getByRole("button", { name: "Start recording" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Record microphone only" }).click();
  await expect.poll(() => calls(page, "start_meeting")).toHaveLength(1);
  expect((await calls(page, "start_meeting"))[0].args.source).toBe("both");
  const alert = page.getByRole("alert").filter({ hasText: "System audio is off" });
  await expect(alert).toContainText("Other people on the call aren't being recorded.");
  await capture(page, "recording-system-off-fog");
  await alert.getByRole("button", { name: "Allow system audio" }).click();
  expect((await calls(page, "permission_assistant")).at(-1).args.kind).toBe("system_audio");
  // Allowed mid-meeting: the recorder joins the system side and clears the notice.
  await page.evaluate(() => window.__testPushSnapshot({ meetingSystemAudio: null, meetingSourceStatus: [] }));
  await expect(alert).toHaveCount(0);
});

for (const theme of ["fog", "slate"]) {
  test(`Microphone only must be dismissed explicitly (${theme})`, async ({ page }) => {
    await startPage(page, { settings: { meetingSource: "microphone" } }, theme);
    await page.getByRole("button", { name: "Start recording" }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("heading")).toHaveText("Record only your microphone?");
    await expect(dialog.locator("#confirmation-detail")).toHaveText(micOnly);
    await expect(dialog.getByRole("button")).toHaveText(["Cancel", "Record microphone only", "Record both sides"]);
    await capture(page, `dialog-microphone-only-${theme}`);
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    expect(await calls(page, "start_meeting")).toHaveLength(0);
    await page.getByRole("button", { name: "Start recording" }).click();
    await dialog.getByRole("button", { name: "Record microphone only" }).click();
    await expect.poll(() => calls(page, "start_meeting")).toHaveLength(1);
    expect((await calls(page, "start_meeting"))[0].args.source).toBe("microphone");
  });
}

test("Windows both sides start without a dialog and no permission is asked", async ({ page }) => {
  await startPage(page, {});
  await expect(page.getByRole("status", { name: "System audio", exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "Start recording" }).click();
  await expect.poll(() => calls(page, "start_meeting")).toHaveLength(1);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await calls(page, "permission_assistant")).toHaveLength(0);
});

test("an unsupported Mac cannot pick system audio and records the microphone after an explicit choice", async ({ page }) => {
  await startPage(page, mac("unsupported"));
  await expect(page.getByRole("button", { name: "Microphone and system audio" })).toBeDisabled();
  await expect(page.getByRole("status", { name: "System audio", exact: true })).toContainText("needs macOS 14.2 or later");
  await page.getByRole("button", { name: "Start recording" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("button")).toHaveText(["Cancel", "Record microphone only"]);
  await dialog.getByRole("button", { name: "Record microphone only" }).click();
  await expect.poll(() => calls(page, "start_meeting")).toHaveLength(1);
  expect((await calls(page, "start_meeting"))[0].args.source).toBe("microphone");
});

test("choosing system audio without permission offers the fix at once", async ({ page }) => {
  await startPage(page, mac("notDetermined", { settings: { meetingSource: "microphone" }, systemAudioAnswer: "ready" }));
  await page.getByRole("button", { name: "Microphone and system audio" }).click();
  await expect.poll(() => calls(page, "permission_assistant")).toHaveLength(1);
  await expect(page.getByRole("status", { name: "System audio", exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "Start recording" }).click();
  await expect.poll(() => calls(page, "start_meeting")).toHaveLength(1);
  expect((await calls(page, "start_meeting"))[0].args.source).toBe("both");
});

async function toMeetingsStep(page, options, theme = "fog") {
  await page.clock.install();
  await installDesktopFixture(page, options);
  await page.goto(`/?page=about&theme=${theme}`);
  await page.getByRole("button", { name: "Take the tour again" }).click();
  const next = async (name = "Continue") => {
    await page.clock.runFor(5000);
    await page.getByRole("button", { name, exact: true }).click();
  };
  await next();
  await next("Agree and continue");
  await next();
  await next();
  await next();
  await next("Use Balanced");
  await expect(page.getByRole("status", { name: "System audio permission" })).toBeVisible();
  await page.clock.runFor(5000);
}

for (const theme of ["fog", "slate"]) {
  test(`setup asks for system audio on a Mac and waits for an answer (${theme})`, async ({ page }) => {
    await toMeetingsStep(page, mac("notDetermined", { systemAudioAnswer: "ready" }), theme);
    const status = page.getByRole("status", { name: "System audio permission" });
    await expect(status).toContainText("System audio: Not allowed yet");
    await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeDisabled();
    await expect(page.getByText("Allow system audio, or choose Not now")).toBeVisible();
    await page.locator(".onboarding-content").evaluate((element) => element.scrollTo({ top: element.scrollHeight }));
    await capture(page, `setup-system-audio-needed-${theme}`);
    await page.getByRole("button", { name: "Allow system audio", exact: true }).click();
    await expect(status).toContainText("System audio: Ready");
    await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeEnabled();
    await capture(page, `setup-system-audio-ready-${theme}`);
  });
}

test("setup Not now is a deliberate, explained choice", async ({ page }) => {
  await toMeetingsStep(page, mac("denied"));
  await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Not now", exact: true }).click();
  await expect(page.getByText(/For now, meetings record only your microphone/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeEnabled();
});

test("Windows setup shows system audio as ready without a permission step", async ({ page }) => {
  await toMeetingsStep(page, { settings: { setupProfile: "balanced" } });
  await expect(page.getByRole("status", { name: "System audio permission" })).toContainText(
    "Windows needs no permission",
  );
  await expect(page.getByRole("button", { name: "Allow system audio" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeEnabled();
  await page.locator(".onboarding-content").evaluate((element) => element.scrollTo({ top: element.scrollHeight }));
  await capture(page, "setup-system-audio-windows-fog");
});
