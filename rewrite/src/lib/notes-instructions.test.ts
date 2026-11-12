import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  instructionLength,
  notesInstructionsError,
} from "./notes-instructions";

test("notes instructions count Unicode code points consistently", () => {
  for (const unit of ["a", "界", "😀"]) {
    assert.equal(instructionLength(unit.repeat(2000)), 2000);
    assert.equal(notesInstructionsError("custom", unit.repeat(2000)), "");
    assert.match(
      notesInstructionsError("custom", unit.repeat(2001)),
      /2,000 characters/,
    );
  }
  assert.match(notesInstructionsError("summary", "a\0b"), /null/);
});
test("only Custom requires nonblank notes instructions", () => {
  assert.match(notesInstructionsError("custom", " \n "), /Add instructions/);
  assert.equal(notesInstructionsError("summary", ""), "");
  assert.equal(notesInstructionsError(null, ""), "");
});
