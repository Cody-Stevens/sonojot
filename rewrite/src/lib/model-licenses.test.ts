import { strict as assert } from "node:assert";
import { test } from "node:test";
import catalog from "../../licenses/download-components.json";
import { modelLicense } from "./model-licenses";

test("every model license has an exact catalog lookup and HTTPS details", () => {
  let count = 0;
  for (const component of catalog.components) {
    if (component.kind !== "model") continue;
    for (const id of component.catalogIds) {
      const license = modelLicense(id);
      assert.ok(license, id);
      assert.ok(license.name.trim(), id);
      assert.equal(license.name, component.license, id);
      assert.equal(
        license.url,
        id === "sonojot-punct-v1" || id === "sonojot-punct-v2"
          ? "https://creativecommons.org/licenses/by-sa/4.0/legalcode.en"
          : component.licenseUrl,
        id,
      );
      const url = new URL(license.url);
      assert.equal(url.protocol, "https:", id);
      assert.equal(url.username, "", id);
      assert.equal(url.password, "", id);
      count++;
    }
  }
  assert.ok(count > 0);
  for (const id of [
    "local-rules",
    "unknown",
    "https://example.com",
    "__proto__",
  ]) {
    assert.equal(modelLicense(id), null);
  }
});

test("punctuation ShareAlike terms remain visible for both versions", () => {
  for (const id of ["sonojot-punct-v1", "sonojot-punct-v2"]) {
    assert.match(modelLicense(id)?.name ?? "", /CC[ -]BY[ -]SA[ -]4\.0/);
  }
});
