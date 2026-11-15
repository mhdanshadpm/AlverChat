import { expect, test, type Page } from '@playwright/test';

/**
 * A heartbeat pong may carry the state a peer declares about itself. The dot wears that
 * declaration and the line under a name quotes it; `status` is the fallback for a peer that
 * sends nothing, which keeps an older build on the network readable.
 */
async function openApp(page: Page) {
  await page.goto('/?mock=1');
  await page.waitForSelector('main.app');
  await page.waitForLoadState('networkidle');
}

const row = (page: Page, name: string) => page.locator('button.ac-chat-row').filter({ hasText: name });

test('a declared presence drives the dot and the line under the name', async ({ page }) => {
  await openApp(page);

  await expect(row(page, 'Deepa R').locator('.ac-avatar__dot--busy')).toHaveCount(1);
  await expect(row(page, 'Sarath Shailan').locator('.ac-avatar__dot--away')).toHaveCount(1);
  // Arjun declares nothing, so his dot comes from liveness alone.
  await expect(row(page, 'Arjun Kumar').locator('.ac-avatar__dot--online')).toHaveCount(1);

  await row(page, 'Sarath Shailan').click();
  await page.waitForSelector('.ac-convo__who');
  await expect(page.locator('.ac-convo__who small')).toContainText('Back at 3:30');
});
