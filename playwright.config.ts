import { defineConfig } from '@playwright/test';

/**
 * Browser-mode screenshots against the mock bridge. Uses the installed Chrome
 * channel so the cached Playwright browser revision never has to match.
 */
export default defineConfig({
  testDir: 'e2e',
  timeout: 90_000,
  workers: 1,
  fullyParallel: false,
  reporter: [['list']],
  use: {
    baseURL: 'http://127.0.0.1:1420',
    channel: 'chrome',
    headless: true,
    viewport: { width: 1440, height: 900 },
    colorScheme: 'light',
    deviceScaleFactor: 1,
  },
  webServer: {
    command: 'pnpm dev',
    url: 'http://127.0.0.1:1420',
    reuseExistingServer: true,
    timeout: 60_000,
  },
});
