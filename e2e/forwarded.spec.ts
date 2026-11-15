import { expect, test, type Page } from '@playwright/test';

/**
 * The "Forwarded" chip is the one bubble state that could never appear in the app: the copy was
 * made, the chip was styled, and nothing on the row said which bubble was a copy. The mock carries
 * a forwarded message now, so these fail if the flag ever stops reaching the bubble.
 */
async function openThread(page: Page, name: string) {
  await page.goto('/?mock=1');
  await page.waitForSelector('main.app');
  await page.waitForLoadState('networkidle');
  await page.locator('button.ac-chat-row').filter({ hasText: name }).first().click();
  await page.waitForSelector('.ac-convo');
}

test('a copied message wears the chip inside its own bubble', async ({ page }) => {
  await openThread(page, 'Arjun Kumar');

  const chip = page.locator('.ac-bubble__forwarded');
  await expect(chip).toHaveCount(1);
  await expect(chip).toContainText('Forwarded');

  const bubble = page.locator('.ac-bubble').filter({ has: chip });
  await expect(bubble).toHaveCount(1);
  await expect(bubble).toContainText('Quarterly numbers from the finance room.');
});

test('a thread nobody has copied into wears no chip', async ({ page }) => {
  await openThread(page, 'Fathima P');
  await expect(page.locator('.ac-bubble__forwarded')).toHaveCount(0);
});
