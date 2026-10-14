// Shared navigation helpers for the desktop-fixture specs. Keep UI wording
// that many specs depend on here, so a renamed page or a new first-run step
// is fixed once instead of in every spec.
import { expect } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";

/** The sidebar landmark; visible only once Terms and first-run setup are done. */
export const mainNavigation = (page) =>
  page.getByRole("navigation", { name: "Main navigation" });

/**
 * Install the fixture, open `path` and wait for the main app shell.
 *
 * The fixture defaults to accepted Terms and a completed Custom profile, so the
 * app opens directly. With `options.firstRun`, this walks first-run setup with
 * `completeFirstRun` first (installing the page clock so step dwell timers can
 * be skipped). Any other case that leaves the app on Terms or setup fails here
 * with a clear message instead of a later selector timeout.
 */
export async function openApp(page, options = {}, path = "/") {
  if (options.firstRun) await page.clock.install();
  await installDesktopFixture(page, options);
  await page.goto(path);
  if (options.firstRun) await completeFirstRun(page);
  await expectAppShell(page);
}

/** Fail fast, and say why, when Terms or first-run setup covers the app. */
export async function expectAppShell(page) {
  const blocker = page
    .getByRole("heading", { name: /^(Welcome to SonoJot|Terms of Use)$/ })
    .or(mainNavigation(page));
  await expect(blocker.first()).toBeVisible();
  if (!(await mainNavigation(page).isVisible()))
    throw new Error(
      "Terms or first-run setup is covering the app. Use openApp(page, { firstRun: true }) or completeFirstRun(page).",
    );
}

/**
 * Walk first-run setup to the app with its default answers: agree to the
 * Terms, accept the microphone check, keep the recommended profile, skip the
 * optional extras and leave from Ready. It drives the footer's primary button
 * rather than naming each step, so new steps need no change here. Needs
 * `page.clock.install()` before `page.goto` (openApp does this).
 */
export async function completeFirstRun(page, { maxSteps = 20 } = {}) {
  const footer = page.locator(".onboarding-footer");
  const primary = footer.locator("button.continue");
  for (let step = 0; step < maxSteps; step += 1) {
    if (await mainNavigation(page).isVisible()) return;
    await expect(primary.or(mainNavigation(page)).first()).toBeVisible();
    if (await mainNavigation(page).isVisible()) return;
    const terms = page.getByRole("checkbox", {
      name: "I agree to the SonoJot Terms of Use",
    });
    if ((await terms.isVisible()) && !(await terms.isChecked()))
      await terms.check();
    // Mac system audio: the step asks for an answer before Continue.
    const notNow = page.getByRole("button", { name: "Not now", exact: true });
    if (await notNow.isVisible()) await notNow.click();
    // Every step has a short dwell; the microphone step falls back to
    // "It's working" after about ten seconds without speech-level input.
    await page.clock.runFor(11_000);
    if (!(await primary.isEnabled())) {
      const working = page.getByRole("button", {
        name: "It's working",
        exact: true,
      });
      if (await working.isVisible()) await working.click();
    }
    await expect(primary).toBeEnabled();
    const label = await primary.textContent();
    await primary.click();
    if (label?.trim() === "Go to SonoJot") break;
  }
  await expect(mainNavigation(page)).toBeVisible();
}

/**
 * Third-party downloads (NVIDIA CUDA, FFmpeg…) ask for license consent first.
 * Agree to every group in the "Before downloading" dialog.
 */
export async function acceptDownloadTerms(page) {
  const dialog = page.getByRole("dialog", { name: "Before downloading" });
  await expect(dialog).toBeVisible();
  for (const box of await dialog.getByRole("checkbox").all()) await box.check();
  await dialog.getByRole("button", { name: "Agree and continue" }).click();
  await expect(dialog).toHaveCount(0);
}

/** Click a sidebar destination by its visible label (for example "Models"). */
export async function goToPage(page, label) {
  await mainNavigation(page)
    .getByRole("button", { name: label, exact: true })
    .click();
  await expect(
    mainNavigation(page).getByRole("button", { name: label, exact: true }),
  ).toHaveAttribute("aria-current", "page");
}

/** Dictation opens on Preferences; recent dictations sit behind "Recent". */
export async function showRecentDictations(page) {
  const recent = page
    .getByRole("group", { name: "Dictation views" })
    .getByRole("button", { name: "Recent", exact: true });
  await recent.click();
  await expect(recent).toHaveAttribute("aria-pressed", "true");
  // The list is a "Recent" card; an opened dictation replaces it untitled.
  return page.getByRole("main");
}

/**
 * Vocabulary opens on your words; "Suggestions from recent dictations" and
 * "Learning from your fixes" are subviews behind its More menu. Returns the
 * subview's card.
 */
export async function openVocabularyView(page, name) {
  await page.locator(".vocabulary-toolbar details.more-menu summary").click();
  await page.getByRole("button", { name: new RegExp(`^${name}`) }).click();
  const card = page.getByRole("region", { name });
  await expect(card).toBeVisible();
  return card;
}

/** Open a meeting from the Meetings list by (part of) its title. */
export async function openMeeting(page, title) {
  await page
    .getByRole("button", {
      name: title instanceof RegExp ? title : new RegExp(title),
    })
    .first()
    .click();
  await expect(page.locator(".meeting-heading")).toBeVisible();
}

/**
 * Open the meeting heading's "More" menu (Rename, Transcribe again…, Speaker
 * labels, Delete meeting…) and return it.
 */
export async function openMeetingMenu(page) {
  const more = page.locator(".meeting-heading details.meeting-more");
  if (!(await more.evaluate((element) => element.open)))
    await more.locator("summary").click();
  await expect(more).toHaveAttribute("open", "");
  return more;
}

/** Open the "Transcribe again" card from the meeting's More menu. */
export async function openTranscribeAgain(page) {
  await (
    await openMeetingMenu(page)
  )
    .getByRole("button", { name: "Transcribe again…", exact: true })
    .click();
  const card = page.getByRole("region", { name: "Transcribe again" });
  await expect(card).toBeVisible();
  return card;
}

/** Open the meeting's "Save a copy" card (document and original audio). */
export async function openSaveCopy(page) {
  const toggle = page
    .locator(".meeting-heading")
    .getByRole("button", { name: "Save a copy", exact: true });
  if ((await toggle.getAttribute("aria-expanded")) !== "true")
    await toggle.click();
  const card = page.getByRole("region", { name: "Save a copy" });
  await expect(card).toBeVisible();
  return card;
}

/** The live activity badge beside the meeting document tabs while it processes. */
export const meetingActivity = (page) =>
  page.locator(".meeting-document-toolbar").getByRole("status");

/**
 * Open a <details> fold by its summary text and return the fold. Pages keep
 * rarely changed controls in folds; their contents are not in the
 * accessibility tree while closed.
 */
export async function openFold(scope, name) {
  const pattern =
    name instanceof RegExp ? name : new RegExp(`^\\s*${name}\\s*$`);
  const summary = scope.locator("summary").filter({ hasText: pattern }).first();
  const fold = summary.locator("xpath=..");
  if (!(await fold.evaluate((element) => element.open))) await summary.click();
  await expect(fold).toHaveAttribute("open", "");
  return fold;
}

/**
 * Dictation's "More options" fold: sound cues, Written form, Shape by type and
 * the models in use.
 */
export const openMoreOptions = (scope) => openFold(scope, "More options");

/**
 * Models page: the model library (Installed / Get more tabs, search, role
 * filter, install and remove) sits in the "Manage downloads" fold.
 */
export const openModelLibrary = (page) => openFold(page, "Manage downloads");

/** Models page: memory, text processing and storage settings. */
export const openPerformance = (page) =>
  openFold(page, "Performance and storage");
