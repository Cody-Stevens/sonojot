import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { TERMS_VERSION, needsTermsAcceptance, termsAcceptance } from "./terms";

test("new, stale and incomplete acceptance records need the prompt", () => {
  const accepted = termsAcceptance(new Date("2026-10-12T12:34:56.000Z"));
  assert.equal(
    needsTermsAcceptance({ termsAcceptedVersion: 0, termsAcceptedAt: null }),
    true,
  );
  assert.equal(
    needsTermsAcceptance({
      ...accepted,
      termsAcceptedVersion: TERMS_VERSION - 1,
    }),
    true,
  );
  assert.equal(
    needsTermsAcceptance({ ...accepted, termsAcceptedAt: null }),
    true,
  );
  assert.equal(
    needsTermsAcceptance({ ...accepted, termsAcceptedAt: "  " }),
    true,
  );
  assert.equal(needsTermsAcceptance(accepted), false);
  assert.equal(
    needsTermsAcceptance({
      ...accepted,
      termsAcceptedVersion: TERMS_VERSION + 1,
    }),
    false,
  );
});

test("acceptance records only the bundled version and the local UTC time", () => {
  assert.deepEqual(termsAcceptance(new Date("2026-10-12T12:34:56.000Z")), {
    termsAcceptedVersion: TERMS_VERSION,
    termsAcceptedAt: "2026-10-12T12:34:56.000Z",
  });
});

test("bundled terms and native capture gate share the UI terms version", () => {
  const terms = readFileSync(
    new URL("../../legal/terms-of-use.md", import.meta.url),
    "utf8",
  );
  const native = readFileSync(
    new URL("../../src-tauri/src/onboarding.rs", import.meta.url),
    "utf8",
  );
  assert.match(terms, new RegExp(`^Version: ${TERMS_VERSION}$`, "m"));
  assert.doesNotMatch(terms, /DRAFT|placeholder|Notes for counsel/);
  assert.match(terms, /Effective October 12, 2026/);
  assert.match(
    native,
    new RegExp(`pub\\(crate\\) const TERMS_VERSION: u32 = ${TERMS_VERSION};`),
  );
});
