import { expect, test } from "@playwright/test";

// Compare painted ink, not just computed CSS: WebKit reported the right colors
// while blend compositing erased the face. A native-text capture is the oracle.
async function checkInk(page, locator, dark, label, testInfo, scroll = true) {
  if (scroll) await locator.scrollIntoViewIfNeeded();
  await page.waitForTimeout(80);
  const box = await locator.boundingBox();
  const clip = {
    x: Math.max(0, box.x - 2),
    y: Math.max(0, box.y - 5),
    width: box.width + 4,
    height: box.height + 12,
  };
  const material = await page.screenshot({ clip });
  await page.evaluate(
    () => (document.documentElement.dataset.typeMaterial = "off"),
  );
  expect(await locator.boundingBox()).toEqual(box);
  const native = await page.screenshot({ clip });
  await page.evaluate(
    () => delete document.documentElement.dataset.typeMaterial,
  );
  const result = await page.evaluate(
    async ({ material, native, dark, bottom }) => {
      async function pixels(base64) {
        const image = new Image();
        image.src = `data:image/png;base64,${base64}`;
        await image.decode();
        const canvas = document.createElement("canvas");
        canvas.width = image.width;
        canvas.height = image.height;
        const ctx = canvas.getContext("2d");
        ctx.drawImage(image, 0, 0);
        return {
          data: ctx.getImageData(0, 0, canvas.width, canvas.height).data,
          width: canvas.width,
          height: canvas.height,
        };
      }
      const [a, b] = await Promise.all([pixels(material), pixels(native)]);
      let ink = 0,
        retained = 0,
        tail = 0,
        tailRetained = 0;
      const isInk = (d, i) =>
        dark
          ? Math.min(d[i], d[i + 1], d[i + 2]) > 155
          : Math.max(d[i], d[i + 1], d[i + 2]) < 135;
      for (let i = 0; i < b.data.length; i += 4)
        if (isInk(b.data, i)) {
          // Edge lighting legitimately changes coverage near antialiasing. Require
          // visible pigment there, rather than identical brightness to flat type.
          ink++;
          const painted = dark
            ? Math.min(a.data[i], a.data[i + 1], a.data[i + 2]) > 110
            : Math.max(a.data[i], a.data[i + 1], a.data[i + 2]) < 155;
          if (painted) retained++;
          if (Math.floor(i / 4 / b.width) >= bottom * b.height) {
            tail++;
            if (painted) tailRetained++;
          }
        }
      return {
        ink,
        retained: retained / ink,
        tail,
        tailRetained: tail ? tailRetained / tail : 1,
      };
    },
    {
      material: material.toString("base64"),
      native: native.toString("base64"),
      dark,
      bottom: (box.height + 5) / clip.height,
    },
  );
  await testInfo.attach(label, { body: material, contentType: "image/png" });
  expect(result.ink, label).toBeGreaterThan(20);
  expect(result.retained, `${label}: lost ink`).toBeGreaterThan(0.85);
  expect(result.tailRetained, `${label}: clipped descender`).toBeGreaterThan(
    0.85,
  );
  return result;
}

test("theme toggle repaints ink without a scroll or reload", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("http://127.0.0.1:4321");
  await page.waitForSelector(".window-text[data-grain-ready]");
  await page.evaluate(() => document.fonts.ready);
  for (const theme of ["basalt", "fog", "basalt"]) {
    await page.locator("[data-theme-toggle]").click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await page.waitForTimeout(250);
    await checkInk(page, page.locator(".window-text"), theme === "basalt", `toggle-${theme}`, testInfo, false);
  }
});

for (const scale of [1, 1.25, 1.5, 2]) {
  test(`website and shared lab retain ink at ${scale}x`, async ({
    browser,
  }, testInfo) => {
    const context = await browser.newContext({
      viewport: { width: 390, height: 844 },
      deviceScaleFactor: scale,
      reducedMotion: "reduce",
    });
    const page = await context.newPage();
    try {
      await page.goto("http://127.0.0.1:4321");
      await page.waitForSelector("[data-grain-ready]");
      await page.evaluate(() => document.fonts.ready);
      for (const theme of ["fog", "basalt"]) {
        await page.evaluate(
          (t) => (document.documentElement.dataset.theme = t),
          theme,
        );
        for (const selector of [
          ".footnote",
          ".transcript",
          "#meetings .section-title",
          ".disclaimer",
          ".site-foot nav",
        ]) {
          await checkInk(
            page,
            page.locator(selector),
            theme === "basalt",
            `${theme}-${selector}`,
            testInfo,
          );
        }
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
        ).toBe(true);
        for (const mode of ["heard", "format", "clean"]) {
          await page.locator(`[data-mode-button=${mode}]`).click();
          await checkInk(
            page,
            page.locator(".transcript"),
            theme === "basalt",
            `${theme}-${mode}`,
            testInfo,
          );
        }
      }
      await page.goto(
        "http://127.0.0.1:1420/src/tests/typography-fixture/index.html",
      );
      await page.waitForSelector(".type-lab-h1[data-grain-ready]");
      await page.evaluate(() => document.fonts.ready);
      for (const theme of ["fog", "slate"]) {
        const panel = page.locator(`.type-lab-panel[data-theme=${theme}]`);
        await panel.locator(".type-lab-h1").evaluate((e) => {
          e.textContent = "shape gyjp";
          e.style.fontSize = "40px";
          e.style.lineHeight = ".9";
        });
        const tail = await checkInk(
          page,
          panel.locator(".type-lab-h1"),
          theme === "slate",
          `${theme}-descenders`,
          testInfo,
        );
        expect(
          tail.tail,
          "fixture must exercise ink outside the line box",
        ).toBeGreaterThan(0);
        for (const treatment of ["flat", "imprint", "emboss"]) {
          await checkInk(
            page,
            panel.locator(`.type-lab-line[data-text-surface=${treatment}]`),
            theme === "slate",
            `${theme}-${treatment}`,
            testInfo,
          );
        }
      }
    } finally {
      await context.close();
    }
  });
}
