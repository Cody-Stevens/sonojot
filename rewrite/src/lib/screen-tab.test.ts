import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  ScreenTabRequests,
  meetingSourceWarning,
  startTabMeeting,
  tabCapture,
  tabCollapseAllowed,
  tabPreview,
  tabRepairPage,
  type ScreenTabIntent,
} from "./screen-tab";
import { presentError } from "./errors";

const opened: ScreenTabIntent = { visible: true, expanded: true, focus: false };
const collapsed: ScreenTabIntent = {
  visible: true,
  expanded: false,
  focus: false,
};
const hidden: ScreenTabIntent = {
  visible: false,
  expanded: false,
  focus: false,
};
const flush = () => new Promise<void>((resolve) => setImmediate(resolve));

test("both one-sided meeting sources explain the missing side", () => {
  assert.equal(meetingSourceWarning("both"), null);
  assert.equal(
    meetingSourceWarning("microphone"),
    "Only your microphone will be recorded. On a Zoom, Teams or Meet call, the other people won't be in the recording.",
  );
  assert.equal(
    meetingSourceWarning("system"),
    "Only sound from this computer will be recorded. Your own voice won't be in the recording.",
  );
});

test("Record both sides waits for the saved setting before starting", async () => {
  const calls: string[] = [];
  let release: (() => void) | undefined;
  const pending = startTabMeeting(
    true,
    async () => {
      calls.push("saving");
      await new Promise<void>((resolve) => {
        release = resolve;
      });
      calls.push("saved");
    },
    async () => {
      calls.push("start");
    },
  );
  assert.deepEqual(calls, ["saving"]);
  release?.();
  await pending;
  assert.deepEqual(calls, ["saving", "saved", "start"]);
});

test("a settings save failure prevents meeting start", async () => {
  let started = false;
  await assert.rejects(
    startTabMeeting(
      true,
      async () => {
        throw new Error("save failed");
      },
      async () => {
        started = true;
      },
    ),
    /save failed/,
  );
  assert.equal(started, false);
});

test("Start anyway preserves the saved source", async () => {
  const calls: string[] = [];
  await startTabMeeting(
    false,
    async () => {
      calls.push("save");
    },
    async () => {
      calls.push("start");
    },
  );
  assert.deepEqual(calls, ["start"]);
});

test("Hide wins over pending hover and collapse requests", async () => {
  const sent: ScreenTabIntent[] = [];
  let release: (() => void) | undefined;
  const queue = new ScreenTabRequests(
    async (intent) => {
      sent.push(intent);
      if (sent.length === 1)
        await new Promise<void>((resolve) => {
          release = resolve;
        });
    },
    (error) => {
      throw error;
    },
  );
  queue.request(opened);
  queue.request(collapsed);
  queue.request(opened);
  queue.request(hidden);
  assert.deepEqual(sent, [opened]);
  release?.();
  await flush();
  assert.deepEqual(sent, [opened, hidden]);
});

test("reversal during collapse runs last and carries explicit keyboard intent", async () => {
  const sent: ScreenTabIntent[] = [];
  let release: (() => void) | undefined;
  const queue = new ScreenTabRequests(
    async (intent) => {
      sent.push(intent);
      if (sent.length === 1)
        await new Promise<void>((resolve) => {
          release = resolve;
        });
    },
    (error) => {
      throw error;
    },
  );
  const focused = { ...opened, focus: true };
  queue.request(collapsed);
  queue.request(opened);
  queue.request(focused);
  release?.();
  await flush();
  assert.deepEqual(sent, [collapsed, focused]);
});

test("a failed geometry request does not strand Hide, and disposal drops pending work", async () => {
  const sent: ScreenTabIntent[] = [];
  const errors: unknown[] = [];
  const queue = new ScreenTabRequests(
    async (intent) => {
      sent.push(intent);
      if (intent.visible) throw new Error("host failure");
    },
    (error) => errors.push(error),
  );
  queue.request(opened);
  queue.request(hidden);
  await flush();
  assert.equal(errors.length, 1);
  assert.deepEqual(sent, [opened, hidden]);
  queue.dispose();
  queue.request(opened);
  await flush();
  assert.equal(sent.length, 2);
});

test("loading already owns capture, errors do not, and meetings retain their action", () => {
  for (const state of ["loading", "listening"] as const)
    assert.equal(tabCapture({ state, meetingRecording: false }), "dictation");
  assert.equal(
    tabCapture({ state: "finishing", meetingRecording: false }),
    "finishing",
  );
  assert.equal(tabCapture({ state: "error", meetingRecording: false }), "idle");
  assert.equal(
    tabCapture({ state: "cleaning", meetingRecording: false }),
    "cleaning",
  );
  assert.equal(
    tabCapture({ state: "listening", meetingRecording: true }),
    "meeting",
  );
});

test("live preview is bounded without rewriting its Unicode suffix", () => {
  assert.equal(tabPreview("Short text."), "Short text.");
  assert.equal(
    tabPreview("Earlier words 🪶 newest words", 14),
    "🪶 newest words",
  );
  const source = "keep these words ".repeat(1000) + "🪶";
  const preview = tabPreview(source);
  assert.equal(Array.from(preview).length, 280);
  assert.ok(preview.endsWith("🪶"));
  assert.ok(!preview.includes("…"));
  assert.ok(source.endsWith(preview));
});

test("a pinned Keep intent rides the queue verbatim without focus", async () => {
  const sent: ScreenTabIntent[] = [];
  const queue = new ScreenTabRequests(
    async (intent) => {
      sent.push(intent);
    },
    (error) => {
      throw error;
    },
  );
  const keep: ScreenTabIntent = {
    visible: true,
    expanded: true,
    focus: false,
    pinned: true,
  };
  queue.request(keep);
  await flush();
  assert.deepEqual(sent, [keep]);
});

test("idle collapse requires an empty, unpinned, non-recording tab", () => {
  const empty = {
    pointerInside: false,
    keyboardInside: false,
    visible: true,
    recording: false,
    temporary: false,
  };
  assert.ok(tabCollapseAllowed(empty));
  assert.ok(!tabCollapseAllowed({ ...empty, temporary: true }));
  assert.ok(!tabCollapseAllowed({ ...empty, pointerInside: true }));
  assert.ok(!tabCollapseAllowed({ ...empty, keyboardInside: true }));
  assert.ok(!tabCollapseAllowed({ ...empty, recording: true }));
  assert.ok(!tabCollapseAllowed({ ...empty, visible: false }));
});

test("microphone and shortcut repairs open their actual settings pages", () => {
  assert.equal(
    tabRepairPage(presentError("Microphone is unavailable").action.id),
    "microphone",
  );
  assert.equal(
    tabRepairPage(presentError("Could not register hotkey").action.id),
    "shortcuts",
  );
  assert.equal(tabRepairPage("meetings"), "meetings");
  assert.equal(tabRepairPage("appearance"), "appearance");
  assert.equal(tabRepairPage("setup"), "setup");
  assert.equal(tabRepairPage("install-media"), "setup");
});
