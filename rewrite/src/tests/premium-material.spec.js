import { expect, test } from "@playwright/test";
import { writeFile } from "node:fs/promises";

const fixture = "/src/tests/premium-fixture/index.html";
const pages = [
  "dictation",
  "microphone",
  "vocabulary",
  "meetings",
  "appearance",
  "shortcuts",
  "setup",
  "about",
];

async function ready(page) {
  await page.locator(".paper-card").first().waitFor();
  await page.evaluate(() => document.fonts.ready);
}

test("brand font, stock and license load from the source graph without fallback requests", async ({
  page,
}) => {
  const failed = [];
  const loaded = [];
  page.on("response", (response) => {
    if (response.status() >= 400)
      failed.push(`${response.status()} ${response.url()}`);
    loaded.push(response.url());
  });
  page.on("requestfailed", (request) => failed.push(request.url()));
  await page.goto(fixture);
  await ready(page);
  expect(
    await page.evaluate(() => document.fonts.check("18px Stationery")),
  ).toBe(true);
  expect(
    loaded.some((url) =>
      url.includes("/src/brand/fonts/SourceSerif4Variable-Roman.ttf.woff2"),
    ),
  ).toBe(true);
  expect(
    loaded.some((url) => url.includes("/src/brand/material/paper-stock.jpg")),
  ).toBe(true);
  await page.goto("/?page=about");
  await ready(page);
  await page.getByText("Read the Source Serif font license").click();
  await expect(page.locator(".license pre")).toContainText(
    "SIL OPEN FONT LICENSE",
  );
  expect(failed).toEqual([]);
  expect(
    loaded.some((url) => /\/public\/assets\/|\/assets\/paper-stock/.test(url)),
  ).toBe(false);
});

test("the shared fixture has measured clearance around rules, selection and corners", async ({
  page,
}, testInfo) => {
  await page.goto(fixture);
  await ready(page);
  const geometry = await page.evaluate(() => {
    const rect = (element) => element.getBoundingClientRect();
    const cards = [...document.querySelectorAll(".paper-card")];
    const row = document.querySelector(".paper-row");
    const control = row.querySelector(".row-control");
    const segments = document.querySelector(".segments");
    const selection = segments.querySelector('[aria-pressed="true"]');
    const frame = document.querySelector(".page-frame");
    const scroll = document.querySelector(".page-content");
    const footer = document.querySelector(".paper-footer");
    const thumb = getComputedStyle(scroll, "::-webkit-scrollbar-thumb");
    const track = getComputedStyle(scroll, "::-webkit-scrollbar-track");
    return {
      type: getComputedStyle(document.documentElement).fontSize,
      cardPadding: getComputedStyle(cards[0]).padding,
      cardGap: rect(cards[1]).top - rect(cards[0]).bottom,
      rowPadding: getComputedStyle(row).paddingBottom,
      controlRuleClearance: rect(row).bottom - rect(control).bottom,
      navigationHeights: [...document.querySelectorAll(".nav-item")].map(
        (element) => rect(element).height,
      ),
      selectionInsetTop: rect(selection).top - rect(segments).top,
      selectionInsetBottom: rect(segments).bottom - rect(selection).bottom,
      scrollInset: {
        top: rect(scroll).top - rect(frame).top,
        right: rect(frame).right - rect(scroll).right,
      },
      footerGap: rect(footer).top - rect(scroll).bottom,
      footerIsOutsideScrollport: !scroll.contains(footer),
      thumbBorder: thumb.borderRightWidth,
      trackBackground: track.backgroundColor,
      trackShadow: track.boxShadow,
    };
  });
  expect(geometry.type).toBe("18px");
  expect(geometry.cardPadding).toBe("27px");
  expect(geometry.cardGap).toBe(36);
  expect(geometry.rowPadding).toBe("18px");
  expect(geometry.controlRuleClearance).toBeGreaterThanOrEqual(18);
  expect(new Set(geometry.navigationHeights).size).toBe(1);
  expect(geometry.selectionInsetTop).toBeGreaterThanOrEqual(9);
  expect(geometry.selectionInsetBottom).toBeGreaterThanOrEqual(9);
  expect(geometry.scrollInset.top).toBeGreaterThanOrEqual(18);
  expect(geometry.scrollInset.right).toBeGreaterThanOrEqual(9);
  expect(geometry.footerGap).toBeGreaterThanOrEqual(9);
  expect(geometry.footerIsOutsideScrollport).toBe(true);
  expect(geometry.thumbBorder).toBe("2px");
  expect(geometry.trackBackground).toBe("rgba(0, 0, 0, 0)");
  expect(geometry.trackShadow).toBe("none");
  const measurementPath = testInfo.outputPath("geometry.json");
  await writeFile(measurementPath, JSON.stringify(geometry, null, 2));
  await testInfo.attach("geometry", {
    path: measurementPath,
    contentType: "application/json",
  });
  await page.screenshot({ path: testInfo.outputPath("shared-fixture.png") });
});

test("switch, buttons and confirmation respond with short motion and restore focus", async ({
  page,
}) => {
  await page.goto(fixture);
  await ready(page);
  const toggle = page.getByRole("switch", {
    name: "Paste when finished",
    exact: true,
  });
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  expect(
    await toggle
      .locator("span")
      .evaluate((element) => getComputedStyle(element).transitionDuration),
  ).toBe("0.14s, 0.14s");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await expect
    .poll(() =>
      toggle
        .locator("span")
        .evaluate((element) => getComputedStyle(element).transform),
    )
    .toBe("matrix(1, 0, 0, 1, 0, 0)");
  const open = page.getByRole("button", {
    name: "Open confirmation",
    exact: true,
  });
  await open.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Keep working", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(open).toBeFocused();
  await expect(page.getByRole("status")).toHaveText("Kept");
  const buttonMotion = await open.evaluate(
    (element) => getComputedStyle(element).transitionDuration,
  );
  expect(
    buttonMotion.split(", ").every((duration) => duration === "0.14s"),
  ).toBe(true);
});

test("reduced motion is static and history focus reveals the full sentence", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(fixture);
  await ready(page);
  const history = page.locator(".history-item").last();
  await history.focus();
  expect(
    await history
      .locator("span")
      .evaluate((element) => getComputedStyle(element).whiteSpace),
  ).toBe("normal");
  const fullText = await history.locator("span").innerText();
  await expect(history).toHaveAttribute("title", fullText);
  const rowClearance = await history.evaluate((element) => {
    const target = element.getBoundingClientRect();
    const row = element.parentElement.getBoundingClientRect();
    return target.top - row.top;
  });
  expect(rowClearance).toBeGreaterThanOrEqual(9);
  expect(
    await page
      .locator(".status-dot.recording")
      .evaluate((element) => getComputedStyle(element).animationName),
  ).toBe("none");
  expect(
    await page
      .locator(".switch > span")
      .first()
      .evaluate((element) => getComputedStyle(element).transitionDuration),
  ).toBe("0s");
  const close = page.getByRole("button", { name: "Close window", exact: true });
  await close.focus();
  await expect(close).toHaveAttribute("title", "Close window");
  expect(
    await close.evaluate(
      (element) => getComputedStyle(element, "::after").content,
    ),
  ).toBe('"Close window"');
  expect(
    await close.evaluate(
      (element) => getComputedStyle(element, "::after").display,
    ),
  ).not.toBe("none");
});

for (const theme of ["fog", "slate"]) {
  for (const material of ["on", "off"]) {
    test(`all seven views share bounded scroll and footer geometry: ${theme}, material ${material}`, async ({
      page,
    }, testInfo) => {
      for (const name of pages) {
        await page.goto(`/?page=${name}&theme=${theme}&material=${material}`);
        await ready(page);
        const geometry = await page.evaluate(() => {
          const scroll = document.querySelector(".page-content");
          const frame = document.querySelector(".page-frame");
          const footer = document.querySelector(".paper-footer");
          return {
            page: {
              width: scroll.clientWidth,
              scrollWidth: scroll.scrollWidth,
              height: scroll.clientHeight,
              scrollHeight: scroll.scrollHeight,
            },
            bodyOverflows: document.body.scrollHeight > innerHeight,
            footerBottom: footer.getBoundingClientRect().bottom,
            scrollBottom: scroll.getBoundingClientRect().bottom,
            frameBottom: frame.getBoundingClientRect().bottom,
            footerTop: footer.getBoundingClientRect().top,
            footerOutside: !scroll.contains(footer),
            theme: document.querySelector(".studio").dataset.theme,
            material: document.querySelector(".studio").dataset.material,
          };
        });
        expect(geometry.page.scrollWidth, name).toBeLessThanOrEqual(
          geometry.page.width + 1,
        );
        expect(geometry.bodyOverflows, name).toBe(false);
        expect(geometry.footerOutside, name).toBe(true);
        expect(
          geometry.footerTop - geometry.scrollBottom,
          name,
        ).toBeGreaterThanOrEqual(9);
        expect(
          geometry.frameBottom - geometry.footerBottom,
          name,
        ).toBeGreaterThanOrEqual(9);
        expect(geometry.theme).toBe(theme);
        expect(geometry.material).toBe(material);
        if (geometry.page.scrollHeight > geometry.page.height) {
          const scroll = page.locator(".page-content");
          await scroll.focus();
          await page.keyboard.press("Control+End");
          await expect
            .poll(() => scroll.evaluate((element) => element.scrollTop))
            .toBeGreaterThan(0);
          const footerBottom = await page
            .locator(".paper-footer")
            .evaluate((element) => element.getBoundingClientRect().bottom);
          expect(footerBottom).toBe(geometry.footerBottom);
          await scroll.evaluate((element) => {
            element.scrollTop = 0;
          });
          await scroll.hover();
          await page.mouse.wheel(0, 500);
          await expect
            .poll(() => scroll.evaluate((element) => element.scrollTop))
            .toBeGreaterThan(0);
        }
        await page.locator(".page-content").evaluate((element) => {
          element.scrollTop = 0;
        });
        await page.screenshot({ path: testInfo.outputPath(`${name}.png`) });
        const measurementPath = testInfo.outputPath(`${name}-geometry.json`);
        await writeFile(measurementPath, JSON.stringify(geometry, null, 2));
        await testInfo.attach(`${name}-geometry`, {
          path: measurementPath,
          contentType: "application/json",
        });
      }
    });
  }
}

for (const width of [900, 680, 390]) {
  test(`shared fixture stays inside a ${width}px browser viewport`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 700 });
    await page.goto(fixture);
    await ready(page);
    const overflow = await page
      .locator(".page-content")
      .evaluate((element) => element.scrollWidth - element.clientWidth);
    expect(overflow).toBeLessThanOrEqual(1);
    const closeEdge = await page
      .getByRole("button", { name: "Close window", exact: true })
      .evaluate((element) => element.getBoundingClientRect().right);
    expect(closeEdge).toBeLessThanOrEqual(width - 9);
    await expect(page.locator(".paper-footer")).toBeInViewport();
    await page.screenshot({
      path: testInfo.outputPath(`fixture-${width}.png`),
    });
  });
}
