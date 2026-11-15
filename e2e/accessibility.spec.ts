import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

/**
 * Accessibility smoke test over the mock UI. Serious and critical findings fail;
 * moderate/minor issues are reported by the log so they can be triaged against
 * the design boards without blocking the suite.
 */
const CATEGORIES = [
  'General',
  'Floating Icon & Bubbles',
  'Notifications',
  'Messages',
  'Calls',
  'Privacy & Network',
  'About',
] as const;

async function audit(page: Page, label: string) {
  const results = await new AxeBuilder({ page }).analyze();
  const blocking = results.violations.filter(
    violation => violation.impact === 'serious' || violation.impact === 'critical',
  );
  if (results.violations.length > 0) {
    console.log(
      `${label}: ${results.violations
        .map(violation => `${violation.id} (${violation.impact}, ${violation.nodes.length})`)
        .join(', ')}`,
    );
    for (const violation of results.violations) {
      for (const node of violation.nodes.slice(0, 6)) {
        console.log(`  ${violation.id} ${node.target.join(' ')} :: ${node.failureSummary?.replace(/\n/g, ' | ')}`);
        if (violation.id === 'label-content-name-mismatch') console.log(`    html: ${node.html}`);
      }
    }
  }
  expect(
    blocking.map(violation => `${violation.id}: ${violation.nodes[0]?.target.join(' ')}`),
    `${label} has serious accessibility violations`,
  ).toEqual([]);
}

async function settle(page: Page) {
  await page.waitForSelector('main.app');
  await page.waitForLoadState('networkidle');
  await page.waitForTimeout(300);
}

test('every main view passes the serious/critical audit', async ({ page }) => {
  await page.goto('/?mock=1');
  await settle(page);
  await audit(page, 'chats');

  await page.getByRole('button', { name: /Arjun Kumar/ }).first().click();
  await settle(page);
  await audit(page, 'conversation');

  for (const screen of ['Groups', 'People', 'Calls'] as const) {
    await page.getByRole('button', { name: screen, exact: true }).click();
    await settle(page);
    await audit(page, screen.toLowerCase());
  }

  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await settle(page);
  for (const category of CATEGORIES) {
    await page
      .locator('nav[aria-label="Settings categories"]')
      .getByRole('button', { name: category, exact: true })
      .click();
    await settle(page);
    await audit(page, `settings ${category}`);
  }

  await page.getByRole('button', { name: 'Open my profile' }).click();
  await settle(page);
  await audit(page, 'profile');

  await page.goto('/?mock=1&scenario=first-run');
  await settle(page);
  await audit(page, 'onboarding');
});
