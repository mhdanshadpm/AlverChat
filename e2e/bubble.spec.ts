import { expect, test, type Page } from '@playwright/test';

/**
 * Chat-bubble regressions seen on the live desktop: a one-word outgoing message
 * pushed its timestamp and delivery tick outside the bubble, and every row drew a
 * selection checkbox even when nothing was being multi-selected.
 */
async function openConversation(page: Page) {
  await page.goto('/?mock=1');
  await page.waitForSelector('main.app');
  await page.getByRole('button', { name: /Arjun Kumar/ }).first().click();
  await page.waitForSelector('.ac-bubble');
}

/** Pixels by which each visible bubble's meta line escapes that bubble. */
function metaOverflow(page: Page) {
  return page.locator('.ac-bubble').evaluateAll((bubbles) =>
    bubbles.reduce<number[]>((escaped, bubble) => {
      const meta = bubble.querySelector<HTMLElement>('.ac-bubble__meta');
      if (!meta) return escaped;
      const outer = bubble.getBoundingClientRect();
      const inner = meta.getBoundingClientRect();
      escaped.push(Math.max(outer.left - inner.left, inner.right - outer.right, inner.bottom - outer.bottom));
      return escaped;
    }, []),
  );
}

test('the timestamp and delivery ticks stay inside the bubble', async ({ page }) => {
  await openConversation(page);
  await page.getByRole('textbox', { name: 'Write a message…' }).fill('hi');
  await page.keyboard.press('Enter');
  await expect(page.locator('.ac-bubble').last()).toContainText('hi');

  expect((await metaOverflow(page)).filter((pixels) => pixels > 1)).toEqual([]);
});

test('checkboxes appear only after the message menu selects one', async ({ page }) => {
  await openConversation(page);
  await expect(page.locator('.ac-bubble__check')).toHaveCount(0);

  await page.locator('.ac-bubble').nth(1).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Select', exact: true }).click();

  await expect(page.getByText('1 selected')).toBeVisible();
  expect(await page.locator('.ac-bubble__check').count()).toBeGreaterThan(0);
  await expect(page.locator('.ac-bubble__check:checked')).toHaveCount(1);

  await page.getByRole('button', { name: 'Clear selection' }).click();
  await expect(page.locator('.ac-bubble__check')).toHaveCount(0);
});
