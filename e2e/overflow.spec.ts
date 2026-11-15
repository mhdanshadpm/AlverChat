import { expect, test, type Page } from '@playwright/test';

/**
 * The timestamp-escaping-the-bubble report is a bug class rather than a one-off: content that
 * leaves its box, or that `overflow: hidden` clips with no ellipsis to say so. This walks the
 * main views at three widths and the three overlay windows at their real sizes, and fails on the
 * offenders, the way the axe audit fails on accessibility ones.
 */
const SCREENS = ['Chats', 'Groups', 'People', 'Calls', 'Settings'] as const;

async function settle(page: Page) {
  await page.waitForSelector('main.app');
  await page.waitForLoadState('networkidle');
}

/** Boxes that leave the viewport, and text clipped without an ellipsis. */
function audit(page: Page, root: string, bothAxes: boolean) {
  return page.evaluate(
    ([rootSelector, vertical]) => {
      const name = (node: Element) =>
        node.className ? `${node.tagName}.${String(node.className).split(' ')[0]}` : node.tagName;
      const width = document.documentElement.clientWidth;
      const height = document.documentElement.clientHeight;
      const escaped: string[] = [];
      const clipped: string[] = [];
      for (const node of Array.from(document.querySelectorAll(rootSelector))) {
        const element = node as HTMLElement;
        const style = getComputedStyle(element);
        if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') continue;
        // Screen-reader-only text is clipped on purpose (`clip-path: inset(50%)`).
        if (style.clipPath !== 'none') continue;
        const box = element.getBoundingClientRect();
        if (box.width === 0 || box.height === 0) continue;
        const text = element.textContent?.trim() ?? '';
        if (!text && !element.querySelector('img,svg')) continue;
        const outHorizontally = box.right > width + 1 || box.left < -1;
        const outVertically = vertical && (box.bottom > height + 1 || box.top < -1);
        if (outHorizontally || outVertically) {
          let reported = true;
          // A tab strip the size of the pane is the design, not a bug; the strip itself
          // is still reported when it is the thing that no longer fits.
          if (outHorizontally) {
            const strip = element.closest('[data-ac-scroll-x]');
            if (strip && strip !== element) {
              const band = strip.getBoundingClientRect();
              if (band.left >= -1 && band.right <= width + 1) reported = false;
            }
          }
          // Same for a list that scrolls down inside a fixed window: the rows below the fold
          // are one wheel click away, not lost. Only the nearest scroll box counts, so a
          // scroller that is itself outside the window is still reported.
          if (reported && outVertically) {
            for (let parent = element.parentElement; parent; parent = parent.parentElement) {
              const overflowY = getComputedStyle(parent).overflowY;
              if (overflowY !== 'auto' && overflowY !== 'scroll') continue;
              const band = parent.getBoundingClientRect();
              if (band.top >= -1 && band.bottom <= height + 1) reported = false;
              break;
            }
          }
          if (reported) {
            escaped.push(
              `${name(element)} left=${Math.round(box.left)} right=${Math.round(box.right)} bottom=${Math.round(box.bottom)}`,
            );
          }
        }
        const hidesHorizontally = style.overflow === 'hidden' || style.overflowX === 'hidden';
        if (
          hidesHorizontally &&
          style.textOverflow !== 'ellipsis' &&
          element.scrollWidth - element.clientWidth > 1 &&
          text.length > 0 &&
          text.length <= 120
        ) {
          clipped.push(`${name(element)} "${text.slice(0, 40)}"`);
        }
      }
      return { escaped: escaped.slice(0, 10), clipped: clipped.slice(0, 10) };
    },
    [root, bothAxes] as [string, boolean],
  );
}

for (const width of [1440, 900, 620]) {
  test(`${width}px: nothing escapes the pane and nothing is clipped silently`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/?mock=1');
    await settle(page);

    // Collect across every screen so one run lists all offenders, not just the first.
    const findings: string[] = [];
    for (const screen of SCREENS) {
      // The chats row carries an unread count in its accessible name, so match by substring.
      await page.locator('nav.rail-nav').getByRole('button', { name: screen }).first().click();
      await settle(page);
      const found = await audit(page, 'main *', false);
      for (const box of found.escaped) findings.push(`${screen}: outside the viewport ${box}`);
      for (const box of found.clipped) findings.push(`${screen}: clipped with no ellipsis ${box}`);
    }
    expect(findings, `${width}px: layout overflow`).toEqual([]);
  });
}

/**
 * The windows Rust creates, at the sizes Rust gives them. `root` is the content that must fit
 * inside the window: the dock is one 84 px icon until a bubble arrives, and `resize_dock` then
 * grows the window to 397×220 so the stack has room — in browser mode there is no window to
 * grow, so the idle frame is measured on the icon alone.
 */
const OVERLAYS = [
  { route: 'dock', width: 84, height: 84, root: '.floating-dock', label: 'dock icon' },
  { route: 'dock', width: 397, height: 220, root: '.dock-stack', label: 'dock with bubbles' },
  { route: 'compact', width: 404, height: 664, root: 'body *', label: 'quick chats' },
  { route: 'menu', width: 264, height: 176, root: '.overlay-menu', label: 'menu' },
] as const;

test('overlay windows keep their content inside the window', async ({ page }) => {
  const findings: string[] = [];
  for (const overlay of OVERLAYS) {
    await page.setViewportSize({ width: overlay.width, height: overlay.height });
    // Same document, different hash does not reload: force a fresh boot per window.
    await page.goto(`/?mock=1#/${overlay.route}`);
    await page.reload();
    await page.waitForLoadState('networkidle');
    expect(await page.locator(overlay.root).count(), `${overlay.label} did not render`).toBeGreaterThan(0);
    const found = await audit(page, overlay.root, true);
    for (const box of found.escaped) findings.push(`${overlay.label}: outside the window ${box}`);
    for (const box of found.clipped) findings.push(`${overlay.label}: clipped with no ellipsis ${box}`);
  }
  expect(findings, 'overlay windows').toEqual([]);
});
