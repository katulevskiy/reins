import { createServer } from 'node:net';

import { test, expect, Agent, Desktop, EMAIL, AGENT_REDIRECT, connectAgent } from './support';

/** What goes wrong during onboarding, and what the user sees then. */

/** A loopback port nothing listens on. */
async function closedPort() {
    const server = createServer();
    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
    const port = (server.address() as { port: number }).port;
    await new Promise((resolve) => server.close(resolve));
    return port;
}

test.describe('connecting an agent', () => {
    test('an email that is not one is refused, by the browser and by the server', async ({ page, request, reins }) => {
        const agent = await new Agent(request, reins.base).register();
        await agent.open(page);
        const email = page.getByLabel('Your account email');
        await email.fill('not-an-email');
        await page.getByRole('button', { name: 'Continue' }).click();
        await expect(email).toHaveJSProperty('validity.valid', false);
        await expect(page).toHaveURL(/\/reins\/oauth\/authorize\?/);

        // A browser that does not check the field: the server answers with the form and an error.
        await page.locator('form').evaluate((form: HTMLFormElement) => { form.noValidate = true; });
        await page.getByRole('button', { name: 'Continue' }).click();
        await expect(page.getByRole('alert')).toHaveText('Enter a valid email address.');
        await expect(page.getByRole('heading', { name: `Connect ${agent.name} to Reins` })).toBeVisible();
        await expect(page.getByLabel('Your account email')).toBeVisible();
    });

    test('an email with no account gets the same waiting page, and no phone is asked', async ({ page, request, reins }) => {
        await reins.phone.createAccount();
        const agent = await new Agent(request, reins.base).register();
        const shown = await agent.startInBrowser(page, 'nobody@example.com');
        expect(Number.isInteger(shown)).toBe(true);
        await expect(page.getByText('This page continues by itself.')).toBeVisible();
        expect(await reins.phone.sync(2)).toEqual([]);
    });

    test('an authorization without a client, or with an unknown one, cannot start', async ({ page, reins }) => {
        await page.goto(`${reins.base}/reins/oauth/authorize?response_type=code`);
        await expect(page.getByRole('heading', { name: 'Cannot connect' })).toBeVisible();
        await expect(page.getByText('The request has no client_id.')).toBeVisible();
        await page.goto(`${reins.base}/reins/oauth/authorize?client_id=no-such-client&response_type=code`);
        await expect(page.getByRole('heading', { name: 'Cannot connect' })).toBeVisible();
        await expect(page.getByLabel('Your account email')).toHaveCount(0);
    });

    test('an unknown or used waiting link has expired', async ({ page, request, reins }) => {
        await page.goto(`${reins.base}/reins/oauth/authorize/wait?session=no-such-session`);
        await expect(page.getByRole('heading', { name: 'Link expired' })).toBeVisible();
        await expect(page.getByText('Start the connection again from your AI app.')).toBeVisible();

        // Once the agent is connected, its waiting link cannot be used again.
        await reins.phone.createAccount();
        const agent = await new Agent(request, reins.base).register();
        const code = await agent.startInBrowser(page);
        const waiting = page.url();
        const item = await reins.phone.waitItem();
        await reins.phone.answerPairing(item.id, true, code, 'Agent');
        await agent.callback(page);
        await page.goto(waiting);
        await expect(page.getByRole('heading', { name: 'Link expired' })).toBeVisible();
    });

    test('a connection denied on the phone sends the browser back to the agent with access_denied', async ({ page, request, reins }) => {
        await reins.phone.createAccount();
        const agent = await new Agent(request, reins.base).register();
        await agent.startInBrowser(page);
        const item = await reins.phone.waitItem();
        await reins.phone.answerPairing(item.id, false);
        const query = await agent.callback(page);
        expect(query.get('error')).toBe('access_denied');
        expect(query.get('error_description')).toBe('The request was denied on the phone.');
        expect(query.get('state')).toBe('state-1');
        expect(query.get('code')).toBeNull();
        expect(await reins.phone.connections()).toEqual([]);
    });

    test('tapping a number the browser does not show cancels the connection', async ({ page, request, reins }) => {
        await reins.phone.createAccount();
        const agent = await new Agent(request, reins.base).register();
        const shown = await agent.startInBrowser(page);
        const item = await reins.phone.waitItem();
        const view = await reins.phone.pairingView(item.id);
        const wrong = view.choices.find((choice) => choice !== shown)!;
        const failure = await reins.phone.failure({ cmd: 'answer_pairing', id: item.id, approve: true, code: wrong, label: 'Agent' });
        expect(failure).toMatch(/does not match the browser/);
        const query = await agent.callback(page);
        expect(query.get('error')).toBe('access_denied');
        expect(query.get('code')).toBeNull();
        expect(await reins.phone.connections()).toEqual([]);
    });

    test('a request the user denies on the phone fails for the agent, which stays connected', async ({ page, request, reins }) => {
        await reins.phone.createAccount();
        const agent = await connectAgent(page, request, reins, 'Agent');
        await reins.phone.stockGmail([['m1', 'Bank <alerts@bank.com>']]);
        const search = agent.tool('gmail_search', { query: 'from:bank' });
        const item = await reins.phone.waitItem();
        await reins.phone.deny(item.id);
        const result = await search;
        expect(result.isError).toBe(true);
        expect(result.content[0].text).toBe('Denied by the user on their Reins device.');
        expect(JSON.stringify(result)).not.toContain('alerts@bank.com');
        expect((await agent.rpc('ping')).status()).toBe(200);
    });
});

test.describe('pairing a computer', () => {
    test('a code that is not a pairing code, or no longer one, is refused on the phone', async ({ request, reins }) => {
        await reins.phone.createAccount();
        expect(await reins.phone.failure({ cmd: 'pairing_by_code', code: 'not a code' }))
            .toContain('That is not a Reins pairing code. Scan the QR code your computer shows.');
        // Well formed, but no computer shows it.
        expect(await reins.phone.failure({ cmd: 'pairing_by_code', code: 'BCDF-GHJK' })).toContain('not found');

        // A used code is gone.
        const desktop = new Desktop(request, reins.base);
        const pairing = await desktop.start();
        const view = await reins.phone.pairingByCode(pairing.user_code);
        await reins.phone.answerPairing(view.id, true, pairing.reins_confirm_code, 'Laptop');
        expect((await desktop.poll(pairing.device_code)).status).toBe(200);
        expect(await reins.phone.failure({ cmd: 'pairing_by_code', code: pairing.user_code })).toContain('not found');
    });

    test('a computer waits until its code is claimed, and an unknown device code has expired', async ({ request, reins }) => {
        const desktop = new Desktop(request, reins.base);
        const pairing = await desktop.start();
        const pending = await desktop.poll(pairing.device_code);
        expect(pending.status).toBe(400);
        expect(pending.body.error).toBe('authorization_pending');
        const unknown = await desktop.poll('no-such-device-code');
        expect(unknown.status).toBe(400);
        expect(unknown.body.error).toBe('expired_token');
    });

    test('a computer whose pairing is denied on the phone is told so', async ({ request, reins }) => {
        await reins.phone.createAccount();
        const desktop = new Desktop(request, reins.base);
        const pairing = await desktop.start();
        const view = await reins.phone.pairingByCode(pairing.user_code);
        await reins.phone.answerPairing(view.id, false);
        const denied = await desktop.poll(pairing.device_code);
        expect(denied.status).toBe(400);
        expect(denied.body.error).toBe('access_denied');
        expect(await reins.phone.connections()).toEqual([]);
    });
});

test.describe('an unreachable server', () => {
    test('the phone cannot create an account on a server that does not answer', async ({ reins }) => {
        const server = `http://127.0.0.1:${await closedPort()}`;
        const failure = await reins.phone.failure({ cmd: 'create_account', email: EMAIL, server });
        expect(failure).toMatch(/network error/i);
    });

    test('a server that goes away while the browser waits shows a connection error', async ({ page, request, reins }) => {
        await reins.phone.createAccount();
        const agent = await new Agent(request, reins.base).register();
        await agent.startInBrowser(page);
        await reins.phone.stopServer();
        await expect(page.reload()).rejects.toThrow(/NS_ERROR_CONNECTION_REFUSED|ERR_CONNECTION_REFUSED/);
        await expect(request.post(`${reins.base}/reins/oauth/register`, { data: {} })).rejects.toThrow(/ECONNREFUSED/);
        expect(page.url()).not.toContain(AGENT_REDIRECT);
    });
});

test('an agent cut off on the phone can pair again, and only the new connection works', async ({ page, request, reins }) => {
    await reins.phone.createAccount();
    const first = await connectAgent(page, request, reins, 'Agent');
    const [connection] = await reins.phone.connections();
    await reins.phone.revoke(connection.id);
    expect(await reins.phone.connections()).toEqual([]);
    expect((await first.rpc('ping')).status()).toBe(401);

    // Pairing again goes through the phone again, as the first time.
    const second = await connectAgent(page, request, reins, 'Agent again');
    expect((await reins.phone.connections()).map((c) => c.label)).toEqual(['Agent again']);
    expect((await second.rpc('ping')).status()).toBe(200);
    expect((await first.rpc('ping')).status()).toBe(401);
});
