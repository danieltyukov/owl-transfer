import { expect, test, type Page } from '@playwright/test';

/*
 * One pass over the built app at each of the two widths.
 *
 * The assertions are about the things only a browser can answer: which
 * navigation is on screen, and whether the panes are reachable from it. Every
 * behaviour underneath is covered by the component tests, which do not need a
 * browser and run in a second.
 */

const phone = (): boolean => test.info().project.name === 'phone';

/*
 * A picture of each layout, kept as an artifact rather than as a baseline to
 * compare against. Pixel comparison across machines is a test of font
 * rasterisation, not of this app; what these are for is looking at.
 */
const shot = (page: Page, name: string): Promise<Buffer> =>
  page.screenshot({
    path: `e2e/screenshots/${test.info().project.name}-${name}.png`,
    fullPage: true,
  });

test('the app opens on the folder and names what is in it', async ({ page }) => {
  await page.goto('/');

  await expect(page.getByRole('heading', { name: 'OwlTransfer', level: 1 })).toBeVisible();
  await expect(page.getByRole('button', { name: /^Photos/ })).toBeVisible();
  await expect(page.getByRole('status')).toContainText('Syncing 1 file');

  await shot(page, 'files');
});

test('navigation swaps between a sidebar and a row of tabs at the breakpoint', async ({ page }) => {
  await page.goto('/');

  const tabs = page.getByRole('navigation', { name: 'Sections' });
  const sidebar = page.getByRole('navigation', { name: 'Main' });

  if (phone()) {
    await expect(tabs).toBeVisible();
    await expect(sidebar).toBeHidden();
  } else {
    await expect(tabs).toBeHidden();
    await expect(sidebar).toBeVisible();
  }
});

test('every pane is reachable from whichever navigation is on screen', async ({ page }) => {
  await page.goto('/');

  const nav = page.getByRole('navigation', { name: phone() ? 'Sections' : 'Main' });

  await nav.getByRole('button', { name: 'Devices' }).click();
  await expect(page.getByRole('heading', { name: 'Devices', level: 1 })).toBeVisible();
  await expect(page.getByRole('region', { name: 'This device' })).toContainText('52734');

  await shot(page, 'devices');

  await nav.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByRole('heading', { name: 'Settings', level: 1 })).toBeVisible();
  await expect(page.getByRole('group', { name: 'Appearance' })).toBeVisible();
});

test('a finger gets the taller row and a control it does not have to hover for', async ({
  page,
}) => {
  await page.goto('/');

  const row = page.getByRole('button', { name: /^Photos/ });
  await expect(row).toBeVisible();
  const height = (await row.boundingBox())!.height;

  // Playwright counts a fully transparent element as visible, so the overflow
  // control has to be measured rather than asked.
  const overflow = page.getByRole('button', { name: 'More for Photos' });
  const opacity = await overflow.evaluate(el => getComputedStyle(el).opacity);

  if (phone()) {
    // `pointer: coarse` only matches on a real touch pointer. Without it the
    // phone screenshots would be desktop sizes at a phone width.
    expect(height).toBeGreaterThanOrEqual(44);
    expect(opacity).toBe('1');
  } else {
    expect(height).toBeLessThan(44);
    expect(opacity).toBe('0');
  }
});

test('the dark theme reaches every ground, not only the page', async ({ page }) => {
  await page.goto('/');

  const nav = page.getByRole('navigation', { name: phone() ? 'Sections' : 'Main' });
  await nav.getByRole('button', { name: 'Settings' }).click();
  await page.getByRole('group', { name: 'Appearance' }).getByRole('button', { name: 'Dark' }).click();

  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  // A token defined only inside the media query would leave this the light
  // value on a machine whose system preference is light.
  const bg = await page.evaluate(() =>
    getComputedStyle(document.body).getPropertyValue('background-color'),
  );
  expect(bg).toBe('rgb(17, 18, 20)');

  await shot(page, 'dark');
});

test('the desktop chrome draws its own title bar and follows the window', async ({ page }) => {
  // `?frame` hands the mock a window frame with no window behind it, which is
  // the only way to look at the chrome before the Tauri shell exists.
  test.skip(phone(), 'the phone activity has chrome of its own');
  await page.goto('/?frame');

  const bar = page.getByRole('banner');
  await expect(bar).toContainText('Owl Transfer');
  await expect(bar).toContainText('Files');

  const controls = page.getByRole('group', { name: 'Window' });
  await expect(controls.getByRole('button', { name: 'Maximize' })).toBeVisible();
  await controls.getByRole('button', { name: 'Maximize' }).click();
  await expect(controls.getByRole('button', { name: 'Restore' })).toBeVisible();

  await shot(page, 'framed');
});
