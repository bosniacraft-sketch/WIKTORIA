// Photograph the interface against the mock (dev/mock.py): node dev/shot.js <window or "main"> <out.png> [port]
// Needs the playwright package (NODE_PATH) and a Chromium; PW_CHROMIUM names its executable.
const { chromium } = require('playwright');
(async () => {
  const [win = 'main', out = 'shot.png', port = '8099'] = process.argv.slice(2);
  const browser = await chromium.launch({ executablePath: process.env.PW_CHROMIUM || undefined });
  const page = await browser.newPage({ viewport: { width: 1600, height: 900 } });
  const errors = [];
  page.on('pageerror', e => errors.push(String(e)));
  page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.waitForSelector('.seat', { timeout: 15000 });
  await page.click('.seat');
  await page.click('#play');
  await page.waitForTimeout(2500);
  if (win !== 'main') {
    await page.evaluate(w => { const b = document.querySelector(`[data-win="${w}"]`); if (b) b.click(); }, win);
    await page.waitForTimeout(1200);
  }
  await page.screenshot({ path: out });
  console.log(errors.length ? 'page errors:\n' + errors.join('\n') : 'no page errors');
  await browser.close();
})();
