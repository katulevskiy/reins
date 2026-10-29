import { test, expect, Desktop } from './support';

/**
 * The page a computer's pairing QR code links to (`/pair?code=`). A phone with the app installed opens the link in the
 * app; this page is what everyone else sees: the code, a button into the app, and where to get the app.
 */

const GET_THE_APP = 'https://app.reins2fa.com/get';

test('the QR link of a new computer shows its pairing code and opens the app', async ({ page, request, reins }) => {
    const pairing = await new Desktop(request, reins.base).start();
    expect(pairing.user_code).toMatch(/^[A-Z]{4}-[A-Z]{4}$/);
    expect(pairing.verification_uri).toBe(`${reins.base}/pair`);
    expect(pairing.verification_uri_complete).toBe(`${reins.base}/pair?code=${pairing.user_code}`);

    await page.goto(pairing.verification_uri_complete);
    await expect(page).toHaveTitle('Connect your computer to Reins');
    await expect(page.getByRole('heading', { name: 'Connect your computer' })).toBeVisible();
    await expect(page.getByLabel('pairing code')).toHaveText(pairing.user_code);
    await expect(page.getByRole('link', { name: 'Open in Reins' }))
        .toHaveAttribute('href', `reins://pair?code=${pairing.user_code}`);
    await expect(page.getByText('scan the QR code with your phone\'s camera instead')).toBeVisible();
});

test('the pairing page tells someone without the app where to get it', async ({ page, request, reins }) => {
    const pairing = await new Desktop(request, reins.base).start();
    await page.goto(pairing.verification_uri_complete);
    await expect(page.getByText('No app yet? Get it, sign in, then scan again.')).toBeVisible();
    await expect(page.getByRole('link', { name: 'Get it' })).toHaveAttribute('href', GET_THE_APP);

    // The links into the app are backed by the app-link documents the phones check.
    const apple = await request.get(`${reins.base}/.well-known/apple-app-site-association`);
    expect(apple.status()).toBe(200);
    expect(apple.headers()['content-type']).toContain('application/json');
    const android = await request.get(`${reins.base}/.well-known/assetlinks.json`);
    expect(android.status()).toBe(200);
    expect(Array.isArray(await android.json())).toBe(true);
});

test.describe('on an Android phone without the app', () => {
    test.use({ userAgent: 'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 Mobile Safari/537.36' });

    test('the pairing page opens the app through an intent with the download page as fallback', async ({ page, request, reins }) => {
        const pairing = await new Desktop(request, reins.base).start();
        await page.goto(pairing.verification_uri_complete);
        const open = page.getByRole('link', { name: 'Open in Reins' });
        await expect(open).toHaveAttribute('href', new RegExp(
            `^intent://pair\\?code=${pairing.user_code}#Intent;scheme=reins;package=com\\.reins2fa\\.app;`
            + `S\\.browser_fallback_url=${encodeURIComponent(GET_THE_APP).replace(/\./g, '\\.')};end$`));
    });
});

for (const [why, url] of [
    ['an invalid pairing code', '/pair?code=not-a-code'],
    ['a pairing code of the wrong shape', '/pair?code=ABCD-EFGH-IJKL'],
    ['no pairing code', '/pair'],
]) {
    test(`a pairing link with ${why} explains how to pair instead of opening the app`, async ({ page, reins }) => {
        await page.goto(`${reins.base}${url}`);
        await expect(page.getByRole('heading', { name: 'Connect your computer' })).toBeVisible();
        await expect(page.getByText('Scan the QR code your computer shows with your phone\'s camera')).toBeVisible();
        await expect(page.getByText('Settings, Connect a computer')).toBeVisible();
        await expect(page.getByLabel('pairing code')).toHaveCount(0);
        await expect(page.getByRole('link', { name: 'Open in Reins' })).toHaveCount(0);
        await expect(page.getByRole('link', { name: 'Get it' })).toHaveAttribute('href', GET_THE_APP);
    });
}
