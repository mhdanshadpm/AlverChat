import { expect, test, type Page } from '@playwright/test';

/**
 * Settings → Calls stores an output device, and `setSinkId` is the only web API that can
 * honour it: Chromium and WebView2 have had it since 2020, WebKit has never shipped it, so
 * this runs in the engine where the control means something. What is proved is the ask —
 * the app telling the element that is playing to move to the stored id — because the id in
 * the fixture belongs to no hardware on this machine.
 */
async function openVoiceChat(page: Page) {
  await page.goto('/?mock=1&scenario=speaker');
  await page.waitForSelector('main.app');
  await page.locator('button.ac-chat-row').filter({ hasText: 'Sarath Shailan' }).click();
  await page.waitForSelector('.ac-bubble__voice-btn');
}

test('a voice note is routed to the output chosen in Settings', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(HTMLMediaElement.prototype, 'setSinkId', {
      configurable: true,
      writable: true,
      value: (deviceId: string) => {
        const store = window as unknown as { __sinks?: string[] };
        (store.__sinks ??= []).push(deviceId);
        return Promise.resolve();
      },
    });
  });
  const routed = () => page.evaluate(() => ((window as unknown as { __sinks?: string[] }).__sinks ?? []).join(','));

  await openVoiceChat(page);
  await expect.poll(routed).toContain('mock-output-1');

  // The same fixture also has to be a recording the browser can load, or the control would
  // be wired to a bubble that never plays.
  await page.getByRole('button', { name: 'Play voice note' }).click();
  await expect(page.getByRole('button', { name: 'Pause voice note' })).toBeVisible();
});
