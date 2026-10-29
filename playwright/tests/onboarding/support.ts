import { test as base, expect, type APIRequestContext, type Page } from '@playwright/test';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { createServer, type Server } from 'node:http';
import { createInterface } from 'node:readline';
import * as path from 'node:path';

/**
 * Support for the onboarding tests: the real Reins server and a simulated phone.
 *
 * The phone is the real phone core (`reins-core`), driven by `crates/reins-e2e/examples/playwright_phone.rs`, which
 * also starts the server binary on a free port with a fresh SQLite database. Build both first:
 *
 *     cargo build --features sqlite --bin vaultwarden
 *     cargo build -p reins-e2e --example playwright_phone
 *
 * `REINS_TEST_SERVER_BINARY` and `REINS_PHONE_DRIVER` override where they are looked for (default: the cargo target
 * directory, `CARGO_TARGET_DIR` or `../target`).
 */

const target = process.env.CARGO_TARGET_DIR ?? path.join(__dirname, '../../../target');
const serverBinary = process.env.REINS_TEST_SERVER_BINARY ?? path.join(target, 'debug/vaultwarden');
const driverBinary = process.env.REINS_PHONE_DRIVER ?? path.join(target, 'debug/examples/playwright_phone');

export const EMAIL = 'first-run@example.com';
export const AGENT_NAME = 'Coding Agent';
/** RFC 7636 appendix B. */
export const VERIFIER = 'dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk';
export const CHALLENGE = 'E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM';

export type Item = { id: string, title: string, subtitle: string, action: string, connection_label: string };
export type PairingView = { id: string, client_name: string, client_host: string, choices: number[] };

/** The simulated phone (and the server it runs): one JSON command per line, one answer per line. */
export class Phone {
    private answers: ((line: string) => void)[] = [];
    private queue: Promise<unknown> = Promise.resolve();
    private stderr = '';

    constructor(private child: ChildProcessWithoutNullStreams) {
        const lines = createInterface({ input: child.stdout });
        lines.on('line', (line) => {
            if (line.startsWith('{')) {
                this.answers.shift()?.(line);
            }
        });
        child.stderr.on('data', (chunk) => { this.stderr = (this.stderr + chunk).slice(-20_000); });
        child.on('exit', (code) => {
            for (const answer of this.answers.splice(0)) {
                answer(JSON.stringify({ error: `the phone driver exited (${code}):\n${this.stderr}` }));
            }
        });
    }

    /** Sends `command`; resolves with `ok`, or rejects with the driver's `error`. */
    call<T = any>(command: Record<string, unknown>): Promise<T> {
        const run = () => new Promise<T>((resolve, reject) => {
            this.answers.push((line) => {
                const reply = JSON.parse(line);
                if ('error' in reply) {
                    reject(new Error(`${command.cmd}: ${reply.error}`));
                } else {
                    resolve(reply.ok as T);
                }
            });
            this.child.stdin.write(JSON.stringify(command) + '\n');
        });
        const result = this.queue.then(run, run);
        this.queue = result.catch(() => undefined);
        return result;
    }

    /** Like `call`, but resolves with the driver's error message (for the negative cases). */
    async failure(command: Record<string, unknown>): Promise<string> {
        try {
            await this.call(command);
        } catch (e) {
            return (e as Error).message;
        }
        throw new Error(`${command.cmd} succeeded`);
    }

    createAccount(email = EMAIL, server?: string) { return this.call({ cmd: 'create_account', email, server }); }
    waitItem(timeoutSecs = 30) { return this.call<Item>({ cmd: 'wait_item', timeout_secs: timeoutSecs }); }
    sync(waitSecs = 0) { return this.call<Item[]>({ cmd: 'sync', wait_secs: waitSecs }); }
    pairingView(id: string) { return this.call<PairingView>({ cmd: 'pairing_view', id }); }
    pairingByCode(code: string) { return this.call<PairingView>({ cmd: 'pairing_by_code', code }); }
    answerPairing(id: string, approve: boolean, code?: number, label?: string) {
        return this.call({ cmd: 'answer_pairing', id, approve, code, label });
    }
    approve(id: string, messages: string[] = []) { return this.call({ cmd: 'approve', id, messages }); }
    deny(id: string) { return this.call({ cmd: 'deny', id }); }
    stockGmail(messages: [string, string][]) { return this.call({ cmd: 'stock_gmail', messages }); }
    connections() { return this.call<{ id: string, label: string }[]>({ cmd: 'connections' }); }
    revoke(id: string) { return this.call({ cmd: 'revoke', id }); }
    stopServer() { return this.call({ cmd: 'stop_server' }); }
    async serverLog() { return (await this.call<{ log: string }>({ cmd: 'server_log' })).log; }

    async close() {
        this.child.stdin.end();
        if (this.child.exitCode === null) {
            await new Promise((resolve) => this.child.once('exit', resolve));
        }
    }
}

export type Reins = { base: string, phone: Phone };

export const test = base.extend<{ reins: Reins }>({
    reins: async ({}, use, testInfo) => {
        const child = spawn(driverBinary, [], {
            env: { ...process.env, REINS_TEST_SERVER_BINARY: serverBinary, RUST_BACKTRACE: '1' },
        });
        const phone = new Phone(child);
        const { base } = await phone.call<{ base: string }>({ cmd: 'start_server', relay_wait: 30, offline: 10 });
        await use({ base, phone });
        if (testInfo.status !== testInfo.expectedStatus) {
            const log = await phone.serverLog().catch(() => '(the server was stopped)');
            await testInfo.attach('server.log', { body: log, contentType: 'text/plain' });
        }
        await phone.close();
        await Agent.closeAll();
    },
});

export { expect };

/** A coding agent's MCP client: dynamic registration, PKCE authorization in the browser, then MCP calls. */
export class Agent {
    clientId = '';
    access = '';
    /** The loopback callback the agent registers, as a CLI agent does: a local server on a free port. */
    redirect = '';
    private static listening: Server[] = [];

    static async closeAll() {
        const servers = Agent.listening.splice(0);
        await Promise.all(servers.map((server) => new Promise((resolve) => server.close(resolve))));
    }

    constructor(private request: APIRequestContext, private base: string, readonly name = AGENT_NAME) {}

    async register() {
        const server = createServer((_request, response) => {
            response.writeHead(200, { 'content-type': 'text/html' });
            response.end('<!DOCTYPE html><title>Agent</title><h1>Back in the agent</h1>');
        });
        await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
        Agent.listening.push(server);
        this.redirect = `http://127.0.0.1:${(server.address() as { port: number }).port}/callback`;
        const r = await this.request.post(`${this.base}/reins/oauth/register`, {
            data: {
                client_name: this.name, redirect_uris: [this.redirect], token_endpoint_auth_method: 'none',
                grant_types: ['authorization_code', 'refresh_token'], response_types: ['code'],
            },
        });
        expect(r.status()).toBe(201);
        this.clientId = (await r.json()).client_id;
        return this;
    }

    authorizeUrl(state = 'state-1') {
        const url = new URL(`${this.base}/reins/oauth/authorize`);
        url.search = new URLSearchParams({
            client_id: this.clientId, redirect_uri: this.redirect, response_type: 'code', code_challenge: CHALLENGE,
            code_challenge_method: 'S256', state, resource: `${this.base}/mcp`,
        }).toString();
        return url.toString();
    }

    /** Opens the authorization in the browser: the page asks for the account email. */
    async open(page: Page) {
        await page.goto(this.authorizeUrl());
        await expect(page.getByRole('heading', { name: `Connect ${this.name} to Reins` })).toBeVisible();
    }

    /** Submits `email`; returns the number the waiting page shows, which the user taps on the phone. */
    async submitEmail(page: Page, email = EMAIL) {
        await page.getByLabel('Your account email').fill(email);
        // Firefox on a loaded CI runner can drop the click before the form submits (the page stays as it was, with no
        // error). Submit again while the form is still the one the agent opened: a second submit is harmless, because
        // the server sends a session that already waits to its waiting page.
        await expect(async () => {
            if (new URL(page.url()).search) {
                await page.getByRole('button', { name: 'Continue' }).click({ timeout: 5_000 });
            }
            await expect(page).toHaveURL(/\/reins\/oauth\/authorize\/wait\?session=/, { timeout: 5_000 });
        }).toPass({ timeout: 30_000 });
        await expect(page.getByRole('heading', { name: 'Approve on your phone' })).toBeVisible();
        return Number(await page.getByLabel('code', { exact: true }).textContent());
    }

    /** [`open`] and [`submitEmail`]. */
    async startInBrowser(page: Page, email = EMAIL) {
        await this.open(page);
        return this.submitEmail(page, email);
    }

    /** Waits for the browser to land back at the agent's callback, and returns its query. */
    async callback(page: Page) {
        await page.waitForURL((url) => url.href.startsWith(this.redirect));
        await expect(page.getByRole('heading', { name: 'Back in the agent' })).toBeVisible();
        return new URL(page.url()).searchParams;
    }

    async exchange(code: string) {
        const r = await this.request.post(`${this.base}/reins/oauth/token`, {
            form: {
                grant_type: 'authorization_code', code, redirect_uri: this.redirect, client_id: this.clientId,
                code_verifier: VERIFIER, resource: `${this.base}/mcp`,
            },
        });
        expect(r.status(), await r.text()).toBe(200);
        this.access = (await r.json()).access_token;
        expect(this.access).toBeTruthy();
    }

    async rpc(method: string, params: Record<string, unknown> = {}) {
        return this.request.post(`${this.base}/mcp`, {
            headers: { Authorization: `Bearer ${this.access}` },
            data: { jsonrpc: '2.0', id: 1, method, params },
            timeout: 90_000,
        });
    }

    async tool(name: string, args: Record<string, unknown>) {
        const r = await this.rpc('tools/call', { name, arguments: args });
        expect(r.status(), await r.text()).toBe(200);
        return (await r.json()).result;
    }
}

/**
 * The whole connection: the agent opens the browser, the user types their email, the phone gets the request and
 * the user taps the number the browser shows. Returns the agent with its tokens.
 */
export async function connectAgent(page: Page, request: APIRequestContext, reins: Reins, label = 'My agent') {
    const agent = await new Agent(request, reins.base).register();
    const code = await agent.startInBrowser(page);
    const item = await reins.phone.waitItem();
    const view = await reins.phone.pairingView(item.id);
    expect(view.choices).toContain(code);
    await reins.phone.answerPairing(item.id, true, code, label);
    const query = await agent.callback(page);
    expect(query.get('state')).toBe('state-1');
    await agent.exchange(query.get('code')!);
    return agent;
}

export const DEVICE_GRANT = 'urn:ietf:params:oauth:grant-type:device_code';

export type DeviceCode = {
    device_code: string, user_code: string, verification_uri: string, verification_uri_complete: string,
    expires_in: number, interval: number, reins_confirm_code: number,
};

/** The desktop app's `reins login`: it asks the server for a pairing code, shows it as a QR code, then polls. */
export class Desktop {
    clientId = '';

    constructor(private request: APIRequestContext, private base: string, readonly name = 'Work laptop') {}

    async start(): Promise<DeviceCode> {
        const registered = await this.request.post(`${this.base}/reins/oauth/register`, {
            data: {
                client_name: this.name, redirect_uris: ['http://127.0.0.1/callback'], token_endpoint_auth_method: 'none',
                grant_types: ['authorization_code', 'refresh_token', DEVICE_GRANT], response_types: ['code'],
            },
        });
        expect(registered.status()).toBe(201);
        this.clientId = (await registered.json()).client_id;
        const r = await this.request.post(`${this.base}/reins/oauth/device_authorization`, {
            form: { client_id: this.clientId, scope: 'mcp', resource: `${this.base}/mcp` },
        });
        expect(r.status(), await r.text()).toBe(200);
        return r.json();
    }

    /** One RFC 8628 token poll: `{ status, body }`. */
    async poll(deviceCode: string) {
        const r = await this.request.post(`${this.base}/reins/oauth/token`, {
            form: { grant_type: DEVICE_GRANT, device_code: deviceCode, client_id: this.clientId },
        });
        return { status: r.status(), body: await r.json() };
    }
}
