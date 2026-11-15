import { expect, test, type Page } from '@playwright/test';

/**
 * A peer photo is not a field of the peer list — it is a picture the other machine agreed
 * to publish. So one row must draw that image while a peer who publishes nothing keeps the
 * initials the design boards show, and the chat header must reuse the bytes the list
 * already read instead of asking for them a second time.
 */
async function openApp(page: Page) {
  await page.goto('/?mock=1');
  await page.waitForSelector('main.app');
  await page.waitForLoadState('networkidle');
}

const row = (page: Page, name: string) => page.locator('button.ac-chat-row').filter({ hasText: name });

test('a peer who publishes a photo is drawn with it', async ({ page }) => {
  await openApp(page);

  await expect(row(page, 'Arjun Kumar').locator('.ac-avatar__img')).toHaveCount(1);
  await expect(row(page, 'Fathima P').locator('.ac-avatar__img')).toHaveCount(0);
  await expect(row(page, 'Fathima P').locator('.ac-avatar')).toContainText('FP');

  await row(page, 'Arjun Kumar').click();
  await page.waitForSelector('.ac-convo__who');
  await expect(page.locator('.ac-convo__who .ac-avatar__img')).toHaveCount(1);
});

test('a group keeps the glyph variant, since a group publishes no photo', async ({ page }) => {
  await openApp(page);

  await expect(row(page, 'Billing Team').locator('.ac-avatar--group')).toHaveCount(1);
  await expect(row(page, 'Billing Team').locator('.ac-avatar__img')).toHaveCount(0);
});
