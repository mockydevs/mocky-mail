const { chromium } = require('/home/don/code/zuri-logistics-reference/node_modules/playwright-core');
(async () => {
  const out = process.argv[2], exe = process.argv[3];
  const browser = await chromium.launch({ executablePath: exe });
  async function page(scheme) {
    const ctx = await browser.newContext({ viewport: { width: 360, height: 640 }, deviceScaleFactor: 3, colorScheme: scheme, isMobile: true, hasTouch: true });
    const p = await ctx.newPage();
    await p.goto('http://localhost:5191/', { waitUntil: 'networkidle' });
    await p.waitForTimeout(1500);
    return p;
  }
  let p = await page('light');
  await p.screenshot({ path: `${out}/1-inbox.png` });
  await p.getByRole('button', { name: /Westlands office/ }).first().click();
  await p.waitForTimeout(1500);
  await p.screenshot({ path: `${out}/2-message.png` });
  p = await page('light');
  await p.getByRole('button', { name: 'New message' }).click();
  await p.waitForTimeout(1000);
  await p.screenshot({ path: `${out}/3-compose.png` });
  p = await page('dark');
  await p.screenshot({ path: `${out}/4-inbox-dark.png` });
  p = await page('light');
  await p.getByRole('button', { name: 'Folders and accounts' }).first().click();
  await p.waitForTimeout(800);
  await p.screenshot({ path: `${out}/5-folders.png` });
  await browser.close();
})().catch(e => { console.error(e.message); process.exit(1); });
