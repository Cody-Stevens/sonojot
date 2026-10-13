import { strict as assert } from "node:assert";
import { after, test } from "node:test";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { previewSnapshot } from "./protocol";

// Only this isolated test process gets a simulated native host. Production has no fixture switch.
Object.defineProperty(globalThis, "window", {
  value: { crypto: globalThis.crypto },
  configurable: true,
});
Object.defineProperty(globalThis, "isTauri", {
  value: true,
  configurable: true,
});
const { call, subscribe } = await import("./api");
after(() => {
  clearMocks();
  Reflect.deleteProperty(globalThis, "window");
  Reflect.deleteProperty(globalThis, "isTauri");
});

test("native command adapter sends exact typed arguments and validates the reply", async () => {
  const calls: { command: string; arguments: unknown }[] = [];
  mockIPC((command, args) => {
    calls.push({ command, arguments: args });
    return previewSnapshot;
  });
  const settings = { ...previewSnapshot.settings, theme: "slate" as const };
  const result = await call("save_settings", { settings });
  assert.deepEqual(calls, [
    { command: "save_settings", arguments: { settings } },
  ]);
  assert.deepEqual(result, previewSnapshot);
  mockIPC(() => ({ state: "listening" }));
  await assert.rejects(call("start_dictation"), /invalid response/);
});

test("native errors remain failures rather than successful UI values", async () => {
  mockIPC(() => {
    throw new Error("The microphone is unavailable.");
  });
  await assert.rejects(call("start_dictation"), /microphone is unavailable/);
});

test("event subscription validates payloads and releases its listener", async () => {
  mockIPC(() => null, { shouldMockEvents: true });
  const states: unknown[] = [];
  const errors: unknown[] = [];
  const unlisten = await subscribe(
    (value) => states.push(value),
    (error) => errors.push(error),
  );
  await emit("app-state", previewSnapshot);
  await emit("app-state", { audioLevel: 0.5 });
  assert.equal(states.length, 1);
  assert.equal(errors.length, 1);
  await unlisten();
  const callbacks: unknown = Reflect.get(
    Reflect.get(window, "__TAURI_INTERNALS__"),
    "callbacks",
  );
  assert.ok(callbacks instanceof Map);
  assert.equal(callbacks.size, 0);
});
