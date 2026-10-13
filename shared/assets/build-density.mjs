// Offline asset generation. Requires the app's existing Playwright dev dependency.
// Run from the repository root: node shared/assets/build-density.mjs
import fs from 'node:fs/promises';
import { chromium } from '../../rewrite/node_modules/@playwright/test/index.mjs';
const stock = await fs.readFile(new URL('../../rewrite/src/brand/material/paper-stock-seamless.png', import.meta.url));
const browser = await chromium.launch({ channel: 'msedge' });
try {
  const page = await browser.newPage();
  const result = await page.evaluate(async source => {
    const image = new Image(); image.src = source; await image.decode();
    const canvas = document.createElement('canvas'); canvas.width = image.width; canvas.height = image.height;
    const ctx = canvas.getContext('2d', { willReadFrequently: true }); ctx.drawImage(image, 0, 0);
    const data = ctx.getImageData(0, 0, canvas.width, canvas.height);
    for (let i = 0; i < data.data.length; i += 4) {
      const l = .2126 * data.data[i] + .7152 * data.data[i + 1] + .0722 * data.data[i + 2];
      const v = Math.max(0, Math.min(255, 128 + (l - 234) * 7));
      data.data[i] = data.data[i + 1] = data.data[i + 2] = v;
    }
    ctx.putImageData(data, 0, 0);
    const plate = document.createElement('canvas'); plate.width = plate.height = 940;
    const target = plate.getContext('2d'); target.imageSmoothingQuality = 'high'; target.drawImage(canvas, 0, 0, 940, 940);
    return plate.toDataURL('image/webp', .92).split(',')[1];
  }, `data:image/png;base64,${stock.toString('base64')}`);
  const bytes = Buffer.from(result, 'base64');
  await fs.writeFile(new URL('./paper-density.webp', import.meta.url), bytes);
  console.log(`Shared density plate: ${bytes.length} bytes, 940 × 940`);
} finally { await browser.close(); }
