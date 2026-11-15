import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';

/**
 * Captures every top-level screen in mock light and dark into design/_compare,
 * ready to sit beside the design boards. Browser mode has no peer service, so
 * these shots prove layout and tokens, not network behavior.
 */
const OUT = join(process.cwd(), 'design', '_compare');
mkdirSync(OUT, { recursive: true });

const CATEGORIES = [
  'General',
  'Floating Icon & Bubbles',
  'Notifications',
  'Messages',
  'Calls',
  'Privacy & Network',
  'About',
] as const;

async function settle(page: Page) {
  await page.waitForSelector('main.app');
  // Fonts and the panel entrance animations finish well inside this.
  await page.waitForLoadState('networkidle');
  await page.waitForTimeout(300);
}

async function useTheme(page: Page, theme: 'light' | 'dark') {
  const main = page.locator('main.app');
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const isDark = await main.evaluate(element => element.classList.contains('dark'));
    if (isDark === (theme === 'dark')) return;
    await page.getByRole('button', { name: 'Toggle theme' }).click();
  }
  throw new Error(`could not switch to the ${theme} theme`);
}

async function capture(page: Page, name: string) {
  await page.screenshot({ path: join(OUT, `${name}.png`), animations: 'disabled' });
}

async function openApp(page: Page, query = '') {
  await page.goto(`/?mock=1${query}`);
  await settle(page);
}

for (const theme of ['light', 'dark'] as const) {
  test(`${theme} screens`, async ({ page }) => {
    await openApp(page);
    await useTheme(page, theme);

    await capture(page, `${theme}-chats`);

    await page.getByRole('button', { name: /Arjun Kumar/ }).first().click();
    await settle(page);
    await capture(page, `${theme}-conversation`);

    for (const screen of ['Groups', 'People', 'Calls'] as const) {
      await page.getByRole('button', { name: screen, exact: true }).click();
      await settle(page);
      await capture(page, `${theme}-${screen.toLowerCase()}`);
    }

    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await settle(page);
    for (const category of CATEGORIES) {
      await page
        .locator('nav[aria-label="Settings categories"]')
        .getByRole('button', { name: category, exact: true })
        .click();
      await settle(page);
      await capture(page, `${theme}-settings-${category.toLowerCase().replace(/[^a-z]+/g, '-')}`);
    }

    await page.getByRole('button', { name: 'Open my profile' }).click();
    await settle(page);
    await capture(page, `${theme}-profile`);

    await page.goto('/?mock=1&scenario=first-run');
    await settle(page);
    await useTheme(page, theme);
    await capture(page, `${theme}-onboarding`);

    expect(await page.locator('main.app').isVisible()).toBe(true);
  });
}
