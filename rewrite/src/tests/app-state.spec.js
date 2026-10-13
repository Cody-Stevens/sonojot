import { expect, test } from "@playwright/test";

test("the final meeting update in a burst is delivered without another event", async ({ page }) => {
  await page.clock.install();
  await page.evaluate(() => window.fixture.apply({ ...window.fixture.snapshot, revision: 20, meetingsRevision: 20 }));
  const first = await page.evaluate(() => window.fixture.meetingRevision);
  await page.clock.fastForward(100);
  await page.evaluate(() => window.fixture.apply({ ...window.fixture.snapshot, revision: 21, meetingsRevision: 21 }));
  expect(await page.evaluate(() => window.fixture.meetingRevision)).toBe(first);
  await page.clock.fastForward(1500);
  expect(await page.evaluate(() => window.fixture.meetingRevision)).toBe(first + 1);
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    window.isTauri = true;
    window.requests = [];
    window.__TAURI_INTERNALS__ = {
      invoke: (command, args) =>
        new Promise((resolve, reject) =>
          window.requests.push({ command, args, resolve, reject }),
        ),
    };
  });
  await page.goto("/src/tests/state-fixture.html");
  await page.waitForFunction(() => window.fixture?.ready);
});

test("capture events enable Stop before a delayed start response; late start cannot undo finishing", async ({
  page,
}) => {
  await page.evaluate(() => {
    void window.fixture.dictation("start");
    void window.fixture.dictation("start");
    window.fixture.apply({
      ...window.fixture.snapshot,
      revision: 2,
      sessionId: 7,
      state: "listening",
    });
    void window.fixture.dictation("stop");
  });
  expect(
    await page.evaluate(() => window.requests.map((r) => r.command)),
  ).toEqual(["start_dictation", "stop_dictation"]);
  await page.evaluate(() => {
    window.requests[1].resolve({
      ...window.fixture.snapshot,
      revision: 3,
      state: "finishing",
    });
  });
  await expect
    .poll(() => page.evaluate(() => window.fixture.snapshot.state))
    .toBe("finishing");
  await page.evaluate(() =>
    window.requests[0].resolve({
      ...window.fixture.snapshot,
      revision: 2,
      state: "listening",
    }),
  );
  await expect
    .poll(() => page.evaluate(() => window.fixture.pending.length))
    .toBe(0);
  expect(await page.evaluate(() => window.fixture.snapshot.state)).toBe(
    "finishing",
  );
});

test("cancel during initialization settles independently and suppresses a late failed start", async ({
  page,
}) => {
  await page.evaluate(() => {
    void window.fixture.dictation("start");
    window.fixture.apply({
      ...window.fixture.snapshot,
      revision: 4,
      sessionId: 8,
      state: "loading",
    });
    void window.fixture.dictation("cancel");
    window.requests[1].resolve({
      ...window.fixture.snapshot,
      revision: 5,
      sessionId: null,
      state: "idle",
    });
  });
  await expect
    .poll(() => page.evaluate(() => window.fixture.snapshot.state))
    .toBe("idle");
  await page.evaluate(() =>
    window.requests[0].reject(new Error("late start failure")),
  );
  await expect
    .poll(() => page.evaluate(() => window.fixture.pending.length))
    .toBe(0);
  expect(await page.evaluate(() => window.fixture.error)).toBe("");
});

test("manual review copy carries its result session and reports stale rejection", async ({
  page,
}) => {
  await page.evaluate(() => {
    window.fixture.apply({
      ...window.fixture.snapshot,
      revision: 6,
      state: "idle",
      resultSessionId: 41,
      recognitionCompleteness: "incomplete",
      recognitionWarnings: ["review required"],
      originalText: "<thö",
      outputText: "<thö",
    });
    void window.fixture.copy(false);
  });
  await expect
    .poll(() => page.evaluate(() => window.requests.length))
    .toBe(1);
  expect(
    await page.evaluate(() => ({
      command: window.requests[0].command,
      args: window.requests[0].args,
    })),
  ).toEqual({
    command: "copy_text",
    args: { original: false, resultSessionId: 41 },
  });
  await page.evaluate(() => {
    window.fixture.apply({
      ...window.fixture.snapshot,
      revision: 7,
      state: "listening",
      sessionId: 42,
      resultSessionId: null,
    });
    window.requests[0].reject(
      new Error(
        "This dictation is no longer current. Review it in Recent dictations.",
      ),
    );
  });
  await expect
    .poll(() => page.evaluate(() => window.fixture.pending.length))
    .toBe(0);
  expect(await page.evaluate(() => window.fixture.error)).toContain(
    "no longer current",
  );
});

test("older snapshots cannot overwrite a newer session or hidden tab", async ({
  page,
}) => {
  await page.evaluate(() => {
    window.fixture.apply({
      ...window.fixture.snapshot,
      revision: 10,
      sessionId: 9,
      state: "listening",
      tabVisible: false,
    });
    window.fixture.apply({
      ...window.fixture.snapshot,
      revision: 9,
      sessionId: 8,
      state: "idle",
      tabVisible: true,
    });
  });
  expect(
    await page.evaluate(() => [
      window.fixture.snapshot.sessionId,
      window.fixture.snapshot.state,
      window.fixture.snapshot.tabVisible,
    ]),
  ).toEqual([9, "listening", false]);
});

test("restart does not discard edits or interrupt a pending action", async ({ page }) => {
  await page.evaluate(async () => {
    window.fixture.snapshot.update.status = { state: "ready", version: "0.23.0", notes: null };
    window.fixture.dirty = true;
    await window.fixture.restartToUpdate();
    window.fixture.dirty = false;
    window.fixture.pending = ["settings"];
    await window.fixture.restartToUpdate();
  });
  expect(await page.evaluate(() => window.requests)).toHaveLength(0);
  await page.evaluate(() => {
    window.fixture.pending = [];
    void window.fixture.restartToUpdate();
    void window.fixture.restartToUpdate();
  });
  expect(await page.evaluate(() => window.requests.map((request) => request.command))).toEqual(["restart_to_update"]);
});
