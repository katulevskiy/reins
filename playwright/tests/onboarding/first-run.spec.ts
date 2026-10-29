import { test, expect, Agent, Desktop, EMAIL, AGENT_NAME, connectAgent } from './support';

/**
 * The first run, end to end: the phone creates the account and becomes its approval device, a coding agent connects
 * through the browser (the user taps the number it shows), and the agent's first request waits for the phone.
 */

test('a new account connects a coding agent and its first request waits for the phone', async ({ page, request, reins }) => {
    // First run on the phone: create the account on this server; the phone is now its approval device.
    await reins.phone.createAccount();
    expect(await reins.phone.sync()).toEqual([]);
    expect(await reins.phone.connections()).toEqual([]);

    // The agent registers itself and opens the browser; the user types their email.
    const agent = await new Agent(request, reins.base).register();
    await agent.open(page);
    await expect(page).toHaveTitle('Connect to Reins');
    await expect(page.getByText('Nothing is shared until you approve it on your phone.')).toBeVisible();
    await expect(page.getByRole('alert')).toHaveCount(0);
    const shown = await agent.submitEmail(page);
    await expect(page.getByText(`Open Reins and tap this number to connect ${AGENT_NAME}:`)).toBeVisible();
    await expect(page.getByText('This page continues by itself.')).toBeVisible();
    expect(shown).toBeGreaterThanOrEqual(10);
    expect(shown).toBeLessThanOrEqual(99);

    // The phone gets the connection request and offers the browser's number among its choices.
    const item = await reins.phone.waitItem();
    expect(item.title).toBe(`Connect ${AGENT_NAME} to Reins?`);
    const view = await reins.phone.pairingView(item.id);
    expect(view.client_name).toBe(AGENT_NAME);
    expect(view.client_host).toBe('127.0.0.1');
    expect(view.choices).toHaveLength(3);
    expect(view.choices).toContain(shown);
    await reins.phone.answerPairing(item.id, true, shown, 'Terminal agent');

    // The waiting page goes back to the agent by itself, with a code the agent exchanges for its tokens.
    const query = await agent.callback(page);
    expect(query.get('state')).toBe('state-1');
    expect(query.get('iss')).toBe(reins.base);
    await agent.exchange(query.get('code')!);
    expect((await reins.phone.connections()).map((c) => c.label)).toEqual(['Terminal agent']);
    const ping = await agent.rpc('ping');
    expect(ping.status()).toBe(200);

    // The first request: the agent searches the user's mail and waits until the phone answers.
    await reins.phone.stockGmail([['m1', 'Bank <alerts@bank.com>'], ['m2', 'Spam <x@spam.example>']]);
    const search = agent.tool('gmail_search', { query: 'from:bank' });
    const request1 = await reins.phone.waitItem();
    expect(request1.title).toBe('Terminal agent wants to search your Gmail');
    expect(request1.connection_label).toBe('Terminal agent');
    await reins.phone.approve(request1.id, ['m1']);
    const result = await search;
    expect(result.isError).toBe(false);
    expect(result.structuredContent.messages.map((m: { id: string }) => m.id)).toEqual(['m1']);
    expect(await reins.phone.sync()).toEqual([]);
});

test('an account made on the web vault signs in on the phone and connects an agent', async ({ page, request, reins }) => {
    // Server and account setup elsewhere first (the web vault's registration), then the phone signs in.
    await reins.phone.call({ cmd: 'sign_in', email: EMAIL });
    const agent = await connectAgent(page, request, reins, 'Web agent');
    expect((await reins.phone.connections()).map((c) => c.label)).toEqual(['Web agent']);
    const ping = await agent.rpc('ping');
    expect(ping.status()).toBe(200);
});

test('a computer pairs by the code its QR link shows, and the phone answers its question', async ({ page, request, reins }) => {
    await reins.phone.createAccount();
    const desktop = new Desktop(request, reins.base);
    const pairing = await desktop.start();
    expect(pairing.expires_in).toBeGreaterThan(0);

    // The QR code links here; the user reads (or the phone scans) the code.
    await page.goto(pairing.verification_uri_complete);
    const shownCode = (await page.getByLabel('pairing code').textContent())!;
    expect(shownCode).toBe(pairing.user_code);
    // The phone claims the code (typed in any case and spacing) and taps the number the computer shows.
    const view = await reins.phone.pairingByCode(shownCode.toLowerCase().replace('-', ' '));
    expect(view.client_name).toBe('Work laptop');
    expect(view.choices).toContain(pairing.reins_confirm_code);
    await reins.phone.answerPairing(view.id, true, pairing.reins_confirm_code, 'Work laptop');

    // The computer's next poll logs it in (once: the grant is gone after it), and it can call the server.
    const done = await desktop.poll(pairing.device_code);
    expect(done.status, JSON.stringify(done.body)).toBe(200);
    expect(done.body.access_token).toBeTruthy();
    const ping = await request.post(`${reins.base}/mcp`, {
        headers: { Authorization: `Bearer ${done.body.access_token}` },
        data: { jsonrpc: '2.0', id: 1, method: 'ping' },
    });
    expect(ping.status()).toBe(200);
    expect((await reins.phone.connections()).map((c) => c.label)).toEqual(['Work laptop']);
    expect((await desktop.poll(pairing.device_code)).body.error).toBe('expired_token');
});
