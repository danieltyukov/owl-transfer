import { defineConfig, devices } from '@playwright/test';

/*
 * The smoke test, which is the only check that runs against a real browser.
 *
 * jsdom cannot answer the one question the phone layout raises: whether the
 * tabs and the sidebar swap at the breakpoint. It applies no CSS, so a media
 * query means nothing to it. This builds the app, serves it, and looks.
 *
 * The app runs on the mock backend here, the same way it does in a browser
 * during development: nothing sets `window.__owlBackend`, so `resolveBackend`
 * hands back the mock.
 */
export default defineConfig({
  testDir: './e2e',
  outputDir: './e2e/results',
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? 'line' : 'list',
  use: {
    baseURL: 'http://127.0.0.1:4173',
    trace: 'retain-on-failure',
  },
  projects: [
    {
      name: 'desktop',
      use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 } },
    },
    {
      // A Pixel-sized portrait window, which is the layout the tabs exist for.
      name: 'phone',
      use: { ...devices['Desktop Chrome'], viewport: { width: 390, height: 844 } },
    },
  ],
  webServer: {
    command: 'npm run build && npm run preview -- --port 4173 --strictPort --host 127.0.0.1',
    url: 'http://127.0.0.1:4173',
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
