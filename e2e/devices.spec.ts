import { expect, test } from '@playwright/test';

/**
 * Settings → Calls can only name a microphone or camera once the engine has handed out
 * labels, which "Detect devices" does by briefly granting camera then microphone. The
 * fake Chromium media devices exercise the same unlock the desktop webview does, so this
 * spec fails if that action ever stops filling the pickers or stops keeping a choice.
 */
test.use({
  launchOptions: {
    args: ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream'],
  },
});

test('detecting devices turns an empty picker into a usable one', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.waitForSelector('main.app');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page
    .locator('nav[aria-label="Settings categories"]')
    .getByRole('button', { name: 'Calls', exact: true })
    .click();

  const microphone = page.locator('select[aria-label="Microphone"]');
  const camera = page.locator('select[aria-label="Camera"]');
  await expect(microphone).toBeVisible();

  // Before anything asks for media, both pickers sit on the engine default. How much the
  // list is worth at that moment depends on the machine's camera/microphone consent state,
  // so the "unlabeled devices are dropped" rule is pinned in vitest instead (see
  // components.test.tsx → deviceOptions) and this test only checks the wiring.
  await expect(microphone).toHaveValue('default');
  await expect(camera).toHaveValue('default');

  await page.getByRole('button', { name: 'Detect devices' }).click();

  await expect
    .poll(() => microphone.evaluate((node) => (node as HTMLSelectElement).options.length), { timeout: 15_000 })
    .toBeGreaterThanOrEqual(2);
  await expect(camera.locator('option')).not.toHaveCount(1);
  await expect(page.getByText(/devices ready/)).toBeVisible();

  const chosen = (await camera.locator('option').nth(1).getAttribute('value')) ?? '';
  expect(chosen).not.toBe('default');
  await camera.selectOption(chosen);
  await expect(camera).toHaveValue(chosen);

  // The toast and the surviving choice are what only "Detect devices" produces: a machine
  // that already hands out labels would satisfy the option counts on its own.
  await page.getByRole('button', { name: 'Detect devices' }).click();
  await expect(page.getByText(/devices ready/)).toBeVisible();
  await expect(camera).toHaveValue(chosen);
});
