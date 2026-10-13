import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { site, platforms } from "./src/site.config.mjs";
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replaceAll("\r\n", "\n");

test("app and website carry the same final terms", () => {
  // The app copy carries a "Version: N" marker line that the website omits.
  assert.equal(read("./src/legal/terms.md"), read("../rewrite/legal/terms-of-use.md").replace(/^Version: \d+\n/m, ""));
  for (const name of ["terms", "privacy", "trademarks"]) {
    assert.doesNotMatch(read(`./src/legal/${name}.md`), /proposed text|Notes for counsel|Before publishing|Implementation note|\*\*Status:/);
    assert.match(read(`./dist/${name}/index.html`), /<h1[^>]*>SonoJot/);
  }
});
test("downloads and source use the public stable destinations", () => {
  assert.equal(site.repoUrl, "https://github.com/Cody-Stevens/sonojot");
  assert.deepEqual(platforms.map(p => site.asset(p.asset)), [
    "https://releases.sonojot.com/download/SonoJot-windows-x64-setup.exe",
    "https://releases.sonojot.com/download/SonoJot-macos-arm64.dmg",
  ]);
  const page = read("./dist/index.html");
  assert.doesNotMatch(page, /MIT licensed|Download 0\.13\.0|WindowsSTTApp/);
  for (const route of ["terms", "privacy", "trademarks"]) assert.ok(page.includes(`href="/${route}"`));
  assert.match(page, /Apache License 2\.0/);
  assert.match(page, /SmartScreen/);
});
