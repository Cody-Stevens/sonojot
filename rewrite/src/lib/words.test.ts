import { strict as assert } from "node:assert";
import { test } from "node:test";
import { parseWord, replacementWord, validHeardForm } from "./words";

test("suggested replacements retain literal arrows in the written form", () => {
  for (const written of ["x->y", "x → y"]) {
    const entry = replacementWord("ex to why", written);
    assert.equal(entry.written, written);
    assert.equal(entry.kind, "replace");
    assert.deepEqual(entry.aliases, ["ex to why"]);
  }
  assert.throws(() => replacementWord("invalid!", "x->y"));
  assert.throws(() => replacementWord("word", ""));
  assert.throws(() => replacementWord("word", "x\ny"));
});

test("explicit replacements confirm their heard phrase and bare words become hints", () => {
  assert.deepEqual(parseWord(" omar key -> Omarchy "), {
    kind: "replace",
    heard: "omar key",
    written: "Omarchy",
    enabled: true,
    aliases: ["omar key"],
    context: [],
    scope: null,
  });
  assert.deepEqual(parseWord("Kubernetes"), {
    kind: "hint",
    heard: "Kubernetes",
    written: "Kubernetes",
    enabled: true,
    aliases: [],
    context: [],
    scope: null,
  });
  assert.equal(parseWord("SonoJot CLI").kind, "hint");
  assert.equal(parseWord("tax sheet → Tacksheet").written, "Tacksheet");
});
test("malformed additions cannot become inactive or ambiguous replacement rules", () => {
  for (const input of [
    "",
    "hello ->",
    "-> hello",
    "a -> b -> c",
    "these five words are too many -> short",
    "a,b -> c",
    "bad\nword",
  ])
    assert.throws(() => parseWord(input));
  assert.equal(validHeardForm("tax sheet"), true);
  assert.equal(validHeardForm("a ! b"), false);
});
