// Synthetic checks for the OpenRouter relay page. Runs the real inline script
// from public/connect/openrouter.html inside node:vm against mocked
// location/history/document — no real auth, no network.
//   node --test connect-openrouter.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const html = readFileSync(
  new URL("./public/connect/openrouter.html", import.meta.url),
  "utf8",
);
const source = html.match(/<script>([\s\S]*?)<\/script>/)?.[1];
assert.ok(source, "inline relay script not found");

const STATE = "a".repeat(64);

function run(search, hash = "") {
  const result = { navigated: null, cleared: null };
  const elements = {};
  const element = (id) =>
    (elements[id] ??= {
      textContent: "",
      hidden: true,
      href: "",
      addEventListener() {},
    });
  vm.runInNewContext(source, {
    URLSearchParams,
    location: {
      search,
      hash,
      pathname: "/connect/openrouter",
      replace: (url) => {
        result.navigated = url;
      },
    },
    history: {
      replaceState: (_state, _title, url) => {
        result.cleared = url;
      },
    },
    document: { getElementById: element },
  });
  return {
    ...result,
    message: elements.message?.textContent ?? "",
    againShown: elements.again?.hidden === false,
    link: elements.continue?.href ?? null,
    detail: elements.detail?.hidden === false ? elements.detail.textContent : null,
  };
}

test("valid parameters relay to the fixed loopback path once", () => {
  const r = run(`?port=50123&state=${STATE}&code=abc-123`);
  assert.equal(
    r.navigated,
    `http://localhost:50123/openrouter/${STATE}?code=abc-123`,
  );
  assert.equal(r.cleared, "/connect/openrouter");
  assert.equal(r.againShown, true);
  assert.equal(r.link, r.navigated);
});

// 0.24.1+: the port rides in the first 4 hex digits of a 68-hex state,
// because OpenRouter now returns only `state` and `code`.
const PORTED = "c3cb" + "b".repeat(64); // 0xc3cb = 50123

test("a 68-hex state carries the port when OpenRouter drops it", () => {
  const r = run(`?state=${PORTED}&code=abc-123`);
  assert.equal(
    r.navigated,
    `http://localhost:50123/openrouter/${PORTED}?code=abc-123`,
  );
});

test("a ported state must agree with an explicit port and stay in range", () => {
  for (const query of [
    `?port=50124&state=${PORTED}&code=abc`, // disagreeing port
    `?port=50123&port=50123&state=${PORTED}&code=abc`, // duplicated port
    `?state=0050${"b".repeat(64)}&code=abc`, // port 80 is below 1024
  ]) {
    assert.equal(run(query).navigated, null, query);
  }
  assert.equal(
    run(`?port=50123&state=${PORTED}&code=abc`).navigated,
    `http://localhost:50123/openrouter/${PORTED}?code=abc`,
  );
});

test("the target host and path are fixed to localhost", () => {
  const code = "a&b=c#d";
  const r = run(
    `?port=65535&state=${STATE}&code=${encodeURIComponent(code)}`,
  );
  const url = new URL(r.navigated);
  assert.equal(url.protocol, "http:");
  assert.equal(url.hostname, "localhost");
  assert.equal(url.port, "65535");
  assert.equal(url.pathname, `/openrouter/${STATE}`);
  assert.equal(url.searchParams.get("code"), code);
});

test("missing, duplicated or out-of-range parameters never navigate", () => {
  const cases = [
    `?port=50123&state=${STATE}`, // no code
    `?port=50123&state=${STATE}&code=a&code=b`, // two codes
    `?port=50123&state=${STATE}&code=`, // empty code
    `?port=50123&state=${STATE}&code=a%20b`, // code with space
    `?port=50123&code=abc`, // no state
    `?port=50123&state=${STATE}&state=${STATE}&code=abc`, // two states
    `?port=50123&state=xyz&code=abc`, // state wrong format
    `?state=${STATE}&code=abc`, // no port
    `?port=50123&port=50124&state=${STATE}&code=abc`, // two ports
    `?port=80&state=${STATE}&code=abc`, // port below range
    `?port=70000&state=${STATE}&code=abc`, // port above range
    `?port=abc&state=${STATE}&code=abc`, // port not numeric
    "", // nothing at all
  ];
  for (const search of cases) {
    const r = run(search);
    assert.equal(r.navigated, null, search);
    assert.equal(r.cleared, "/connect/openrouter", search);
    assert.equal(r.againShown, false, search);
  }
});

test("any error parameter is a failed authorization, never a relay", () => {
  for (const error of ["error=access_denied", "error=a&error=b"]) {
    const r = run(`?port=50123&state=${STATE}&code=abc&${error}`);
    assert.equal(r.navigated, null, error);
    assert.match(r.message, /did not complete sign-in/);
  }
});

test("a code appended with a second question mark still relays", () => {
  const r = run(`?port=50123&state=${STATE}?code=abc-123`);
  assert.equal(
    r.navigated,
    `http://localhost:50123/openrouter/${STATE}?code=abc-123`,
  );
});

test("a rejected link shows the address shape but never its values", () => {
  const r = run(`?port=50123&state=${STATE}&other=secret-value`, "#code=xyz");
  assert.equal(r.navigated, null);
  assert.equal(
    r.detail,
    "Missing or invalid: code. Received: ?port=50123&state=[64]&other=[12]#code=[3].",
  );
  assert.doesNotMatch(r.detail, /secret-value|xyz|aaaa/);
});

test("the shape keeps odd separators and masks a non-numeric port", () => {
  const r = run(`?state=${STATE}?code=abc&port=12ab`);
  assert.equal(
    r.detail,
    "Missing or invalid: port. Received: ?state=[64]?code=[3]&port=[4].",
  );
});
