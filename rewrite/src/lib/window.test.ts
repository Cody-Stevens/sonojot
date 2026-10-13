import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  controlWindow,
  dispatchWindowAction,
  headerGesture,
  type WindowAction,
} from "./window";

test("window actions invoke exactly the requested native method", async () => {
  const called: WindowAction[] = [];
  const target = {
    minimize: async () => {
      called.push("minimize");
    },
    toggleMaximize: async () => {
      called.push("toggleMaximize");
    },
    close: async () => {
      called.push("close");
    },
    startDragging: async () => {
      called.push("startDragging");
    },
  };
  const actions: WindowAction[] = [
    "minimize",
    "toggleMaximize",
    "close",
    "startDragging",
  ];
  for (const action of actions) await dispatchWindowAction(target, action);
  assert.deepEqual(called, actions);
  await assert.rejects(
    dispatchWindowAction(
      {
        ...target,
        close: async () => {
          throw new Error("Window did not close");
        },
      },
      "close",
    ),
    /did not close/,
  );
});
test("header gestures exclude interactive controls and nonprimary clicks", () => {
  assert.equal(headerGesture(0, 1, false), "drag");
  assert.equal(headerGesture(0, 2, false), "maximize");
  assert.equal(headerGesture(0, 1, true), null);
  assert.equal(headerGesture(0, 2, true), null);
  assert.equal(headerGesture(2, 1, false), null);
});
test("browser preview cannot pretend to control a native window", async () => {
  await assert.rejects(controlWindow("close"), /desktop app/);
});
