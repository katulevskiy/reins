import { defineConfig } from '@playwright/test';

/**
 * The onboarding tests (`tests/onboarding`): the real Reins server and a simulated phone, no docker.
 * See `tests/onboarding/support.ts` for the two binaries they need, then:
 *
 *     npx playwright test -c onboarding.config.ts
 */
export default defineConfig({
    testDir: './tests/onboarding',
    fullyParallel: false,
    forbidOnly: !!process.env.CI,
    retries: 0,
    workers: 1,
    reporter: [['list'], ['html', { open: 'never', outputFolder: 'playwright-report' }]],
    timeout: 120 * 1000,
    expect: { timeout: 20 * 1000 },
    use: {
        browserName: 'firefox',
        locale: 'en-GB',
        actionTimeout: 20 * 1000,
        navigationTimeout: 20 * 1000,
        trace: 'retain-on-failure',
        viewport: { width: 1080, height: 720 },
    },
    projects: [{ name: 'onboarding', testMatch: '*.spec.ts' }],
});
