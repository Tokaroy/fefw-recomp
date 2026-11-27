// Regenerates the README's launcher screenshots (docs/screenshots/): serves the frontend with Vite, with mock.js in
// place of the Rust side, and captures each shot below in headless Chromium.
//
//   npm run screenshots
//
// Needs the browser once: npx playwright install chromium
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
const outDir = fileURLToPath(new URL('../../docs/screenshots/', import.meta.url));

// the window size in tauri.conf.json
const VIEWPORT = { width: 1080, height: 720 };

/**
 * `scenario` is a state mock.js knows. `click` opens what to show, in order. `ready` is a selector that appears once the
 * page has everything it asks the backend for. `scrollTo` brings an element to the top of the page area.
 */
const SHOTS = [
  { file: 'launcher-play.png', scenario: 'ready', ready: '#play:enabled' },
  { file: 'launcher-setup.png', scenario: 'building', click: ['[data-page="setup"]'], ready: '#steps-card progress', scrollTo: '#steps-card' },
  {
    file: 'launcher-settings.png',
    scenario: 'ready',
    click: ['[data-page="settings"]', '[data-tab="graphics"]'],
    ready: '#f-upscaler',
  },
];

/** Puts mock.js ahead of src/main.js in index.html. */
const mockBackend = {
  name: 'mock-backend',
  transformIndexHtml: () => [{ tag: 'script', attrs: { type: 'module', src: '/screenshots/mock.js' }, injectTo: 'head' }],
};

async function capture(browser, base, shot) {
  const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 2, colorScheme: 'dark', locale: 'en-US' });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  try {
    await page.goto(`${base}?scenario=${shot.scenario}`, { waitUntil: 'networkidle' });
    for (const selector of shot.click ?? []) await page.click(selector);
    await page.waitForSelector(shot.ready);
    await page.waitForLoadState('networkidle');
    // a font that fails to load would leave the picture in a fallback face
    const badFonts = await page.evaluate(async () => {
      await document.fonts.ready;
      return [...document.fonts].filter((f) => f.status === 'error').map((f) => f.family);
    });
    if (badFonts.length) errors.push(`fonts that did not load: ${[...new Set(badFonts)].join(', ')}`);
    if (shot.scrollTo) {
      await page.locator(shot.scrollTo).evaluate((el) => {
        el.style.scrollMarginTop = 'var(--space-6)'; // the page's own top padding
        el.scrollIntoView({ block: 'start' });
      });
    }
    await page.mouse.move(0, 0); // no hover styles
    await page.evaluate(() => document.activeElement?.blur()); // no focus ring
    const unmocked = await page.evaluate(() => window.__unmocked);
    if (unmocked.length) errors.push(`commands without a reply in mock.js: ${[...new Set(unmocked)].join(', ')}`);
    if (errors.length) throw new Error(errors.join('\n'));
    await page.screenshot({ path: outDir + shot.file, animations: 'disabled' });
    console.log(`  ${shot.file}`);
  } catch (e) {
    throw new Error(`${shot.file}: ${[...errors, e.message].join('\n')}`);
  } finally {
    await context.close();
  }
}

const server = await createServer({
  root,
  logLevel: 'warn',
  plugins: [mockBackend],
  // its own port, so a running `tauri dev` is left alone
  server: { port: 4179, strictPort: false },
  // found up front: a dependency discovered while a page loads makes Vite reload it
  optimizeDeps: { entries: ['index.html', 'screenshots/mock.js'] },
});
let browser;
try {
  await server.listen();
  browser = await chromium.launch();
  for (const shot of SHOTS) await capture(browser, server.resolvedUrls.local[0], shot);
} finally {
  await browser?.close();
  await server.close();
}
