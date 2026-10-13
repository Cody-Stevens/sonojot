import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { legalLink, renderLegalMarkdown } from "./legal-markdown";

test("legal text escapes HTML and attribute delimiters", () => {
  const html = renderLegalMarkdown(
    '<script>alert("x")</script> & <img onerror="x">',
  );
  assert.ok(!html.includes("<script>"));
  assert.match(html, /&lt;img onerror=&quot;x&quot;&gt;/);
});
test("only web links become external anchors", () => {
  assert.equal(legalLink("javascript:alert(1)"), null);
  assert.equal(legalLink("file:///secret"), null);
  assert.equal(legalLink("https://user:pass@example.com"), null);
  assert.equal(legalLink("http://sonojot.com/privacy"), null);
  assert.match(
    renderLegalMarkdown("[Privacy](https://sonojot.com/privacy?a=1&b=2)"),
    /href="https:\/\/sonojot.com\/privacy\?a=1&amp;b=2" target="_blank" rel="noopener noreferrer"/,
  );
  assert.ok(!renderLegalMarkdown("[bad](data:text/html,test)").includes("<a"));
});
test("headings, paragraphs, lists, bold and code retain structure", () => {
  const html = renderLegalMarkdown(
    "# Terms\n\nOne\nparagraph.\n\n- **First**\n- **`Second`**\n\n1. Third",
  );
  assert.match(html, /<h2 data-text-surface="emboss">Terms<\/h2>/);
  assert.match(html, /<p>One paragraph.<\/p>/);
  assert.match(
    html,
    /<ul>\n<li><strong>First<\/strong><\/li>\n<li><strong><code>Second<\/code><\/strong><\/li>\n<\/ul>/,
  );
  assert.match(html, /<ol>\n<li>Third<\/li>\n<\/ol>/);
});
test("every rendered legal link is one the desktop app agrees to open", () => {
  const read = (path: string) =>
    readFileSync(new URL(path, import.meta.url), "utf8");
  const allowed = new Set(
    [
      ...read("../../src-tauri/src/legal_links.rs")
        .split("fn checked_url")[1]
        .split("_ =>")[0]
        .matchAll(/"(https:[^"]+)"/g),
    ].map((match) => match[1]),
  );
  const links = [
    ...renderLegalMarkdown(read("../../legal/terms-of-use.md")).matchAll(
      /<a href="([^"]+)"/g,
    ),
    ...read("../components/LegalLinks.svelte").matchAll(/url: "([^"]+)"/g),
  ].map((match) => match[1]);
  assert.ok(links.length >= 3);
  for (const link of links) assert.ok(allowed.has(link), link);
});
