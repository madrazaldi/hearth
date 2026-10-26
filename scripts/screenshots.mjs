// Regenerates the README screenshots in docs/screenshots from the scripted mock.
// Start `bun run dev` first, then `bun run screenshots`. Uses an installed Chrome;
// point HEARTH_CHROME at another Chromium-based browser's binary to use that instead.

import { chromium } from 'playwright-core';

const base = process.env.HEARTH_URL ?? 'http://localhost:1420/';
const out = new URL('../docs/screenshots/', import.meta.url);

// name, mock query params, and how far to scroll the message list up so the
// shot shows conversation rather than the media at the bottom of #hearth.
const shots = [
  ['chat-dark', '', 700],
  ['chat-light', 'theme=light', 700],
  ['columns', 'layout=columns', 700],
  ['search', 'view=search'],
  ['settings', 'view=settings'],
  ['switcher', 'view=switcher'],
  ['channels', 'view=channels'],
];

const exe = process.env.HEARTH_CHROME;
const browser = await chromium.launch(exe ? { executablePath: exe } : { channel: 'chrome' });
const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2, colorScheme: 'dark' });
for (const [name, query, scroll] of shots) {
  const page = await ctx.newPage();
  await page.goto(`${base}?update=none${query ? `&${query}` : ''}`, { waitUntil: 'load' });
  // The mock replays its session and opens views on timers.
  await page.waitForTimeout(2500);
  if (scroll) {
    await page.evaluate((d) => (document.querySelector('.scroller').scrollTop -= d), scroll);
    await page.waitForTimeout(800);
  }
  await page.screenshot({ path: new URL(`${name}.png`, out).pathname });
  await page.close();
  console.log(name);
}
await browser.close();
