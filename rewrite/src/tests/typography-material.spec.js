import { expect, test } from "@playwright/test";

test("material type keeps live text and layout across paper themes and raster scales", async ({
  browser,
}, testInfo) => {
  for (const deviceScaleFactor of [1, 1.25, 1.5, 2]) {
    const context = await browser.newContext({
      viewport: { width: 1220, height: 860 },
      deviceScaleFactor,
      reducedMotion: "reduce",
    });
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    for (const theme of ["fog", "slate"]) {
      await page.goto(`http://127.0.0.1:1420/?theme=${theme}`);
      const title = page.locator(".page-heading h1");
      await expect(title).toHaveAttribute("data-grain-ready", "");
      await page.evaluate(() => document.fonts.ready);
      const before = await title.boundingBox();
      const text = await title.textContent();
      const selected = await title.evaluate((element) => {
        const range = document.createRange();
        range.selectNodeContents(element);
        const selection = window.getSelection();
        selection.removeAllRanges();
        selection.addRange(range);
        const value = selection.toString();
        selection.removeAllRanges();
        return value;
      });
      expect(selected).toBe(text);
      await title.screenshot({
        path: testInfo.outputPath(`title-${theme}-${deviceScaleFactor}.png`),
      });
      await page
        .locator(".studio")
        .evaluate((element) => element.setAttribute("data-material", "off"));
      expect(await title.boundingBox()).toEqual(before);
      const flat = await title.evaluate((element) => {
        const style = getComputedStyle(element);
        return {
          filter: style.filter,
          shadow: style.textShadow,
          fill: style.webkitTextFillColor,
          color: style.color,
          image: style.backgroundImage,
        };
      });
      expect(flat.filter).toBe("none");
      expect(flat.shadow).toBe("none");
      expect(flat.image).toBe("none");
      expect(flat.fill).toBe(flat.color);
      expect(await title.textContent()).toBe(text);
    }
    expect(errors).toEqual([]);
    await context.close();
  }
});

test("grain remains registered to the stationary app stock during scrolling", async ({
  page,
}) => {
  await page.goto("/?page=appearance");
  const title = page.locator(".page-heading h1");
  await expect(title).toHaveAttribute("data-grain-ready", "");
  const before = await title.evaluate((element) =>
    parseFloat(element.style.getPropertyValue("--type-stock-y")),
  );
  const moved = await page.locator(".page-content").evaluate((element) => {
    element.scrollTop = 90;
    return element.scrollTop;
  });
  expect(moved).toBeGreaterThan(0);
  await expect
    .poll(() =>
      title.evaluate((element) =>
        parseFloat(element.style.getPropertyValue("--type-stock-y")),
      ),
    )
    .toBeCloseTo(before + moved, 1);
  // The renderer has no idle frame loop and should stop writing after settling.
  const writes = await page.evaluate(async () => {
    await new Promise((resolve) => setTimeout(resolve, 150));
    let count = 0;
    const observer = new MutationObserver((records) => {
      count += records.length;
    });
    observer.observe(document.querySelector(".page-heading h1"), {
      attributes: true,
    });
    await new Promise((resolve) => setTimeout(resolve, 250));
    observer.disconnect();
    return count;
  });
  expect(writes).toBe(0);
});

test("contrast preferences restore solid text without filters", async ({
  page,
}) => {
  await page.emulateMedia({ forcedColors: "active" });
  await page.goto("/");
  const title = page.locator(".page-heading h1");
  await expect(title).toBeVisible();
  const style = await title.evaluate((element) => {
    const css = getComputedStyle(element);
    return {
      filter: css.filter,
      fill: css.webkitTextFillColor,
      color: css.color,
      shadow: css.textShadow,
    };
  });
  expect(style.filter).toBe("none");
  expect(style.shadow).toBe("none");
  expect(style.fill).toBe(style.color);
});
