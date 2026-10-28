import { defineConfig } from '@playwright/test';

// Each test file starts its own devbridge on a temp data dir (see fixtures.ts).
export default defineConfig({
  testDir: './tests',
  timeout: 60_000,
  workers: 1,
  reporter: [['list']],
  use: { browserName: 'chromium', viewport: { width: 1200, height: 780 }, locale: 'ru-RU' },
});
