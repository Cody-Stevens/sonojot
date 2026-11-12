import { strict as assert } from "node:assert";
import { test } from "node:test";
import { safeShortcut } from "./shortcut";

test("global shortcuts reject typing and navigation keys without a command modifier", () => {
  for (const value of [
    "a",
    "A",
    "Tab",
    "Enter",
    "Space",
    "ArrowLeft",
    "Shift+A",
    "Shift+Tab",
    "Ctrl",
    "Ctrl+",
    "+A",
    "F0",
    "F25",
    "Ctrl+\nA",
  ])
    assert.equal(safeShortcut(value), false, value);
  for (const value of [
    "Ctrl+Space",
    "Alt+Shift+D",
    "Super+A",
    "CommandOrControl+K",
    "F1",
    "F24",
    "Shift+F10",
  ])
    assert.equal(safeShortcut(value), true, value);
});
