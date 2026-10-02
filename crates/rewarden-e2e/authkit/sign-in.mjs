// Plays the person at the browser for a live WorkOS AuthKit sign-in (scripts/workos-live.sh): opens the server's
// /identity/connect/authorize URL in headless Chrome, signs in on AuthKit's hosted page with an email and password
// (a WorkOS test user), and prints the URL the server finally sends the browser to (the app's callback), which
// Chrome itself cannot open.
//
//   AUTHKIT_EMAIL=... AUTHKIT_PASSWORD=... node sign-in.mjs <authorize url> <callback scheme>
//
// Needs playwright-core next to it (npm install playwright-core) and Google Chrome.
import { chromium } from 'playwright-core';

const [url, scheme] = process.argv.slice(2);
const email = process.env.AUTHKIT_EMAIL;
const password = process.env.AUTHKIT_PASSWORD;
if (!url || !scheme || !email || !password) {
  console.error('usage: AUTHKIT_EMAIL=... AUTHKIT_PASSWORD=... node sign-in.mjs <authorize url> <callback scheme>');
  process.exit(2);
}

const browser = await chromium.launch({
  channel: 'chrome',
  headless: true,
  args: ['--disable-blink-features=AutomationControlled'],
});
try {
  const page = await browser.newPage();
  let callback;
  page.on('response', (response) => {
    const location = response.headers()['location'];
    if (location && location.startsWith(`${scheme}:`)) callback = location;
  });
  await page.goto(url);
  await page.getByLabel('Email').fill(email);
  await page.getByRole('button', { name: 'Continue with email' }).click();
  await page.getByLabel('Password').fill(password);
  await page.getByRole('button', { name: /sign in|continue/i }).first().click();
  for (let i = 0; i < 60 && !callback; i++) await page.waitForTimeout(500);
  if (!callback) {
    await page.screenshot({ path: process.env.AUTHKIT_SCREENSHOT || '/tmp/authkit-failed.png' });
    throw new Error(`no callback; the browser is at ${page.url()}`);
  }
  console.log(`CALLBACK ${callback}`);
} finally {
  await browser.close();
}
