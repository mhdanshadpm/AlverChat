import { expect, test } from '@playwright/test';

/**
 * Settings → Opacity only reaches the floating icon if the value travels through the
 * overlay snapshot, and the dock window reads that snapshot and nothing else: until the
 * slider was wired into it the icon rendered fully opaque whatever the setting said.
 * This is the check that would have caught that.
 */
test('the dock fades by the stored opacity', async ({ page }) => {
  await page.goto('/?mock=1#/dock');
  await page.reload();

  const stack = page.locator('.dock-stack');
  await expect(stack).toBeVisible();

  // The stored value is 90 %, handed over as a percentage and divided by the dock.
  await expect(stack).toHaveCSS('opacity', '0.9');
});

/**
 * The board's Do-Not-Disturb frame: an orange ring, a minus chip, and no bubbles at all.
 * Quiet is decided by the engine (hand-muted or inside the scheduled window) and handed
 * over as one boolean, so the ring and the suppressed chime cannot disagree.
 */
test('a quiet device puts the dock in its Do-Not-Disturb state', async ({ page }) => {
  await page.goto('/?mock=1&scenario=dnd#/dock');
  await page.reload();

  const dock = page.locator('.floating-dock');
  await expect(dock).toBeVisible();
  await expect(dock).toHaveClass(/dock-mood-dnd/);
  await expect(dock.locator('.dock-state-chip.dnd')).toHaveCount(1);
  // Quiet owns the bottom-right corner, so the borrowed face drops its presence dot there.
  await expect(dock.locator('.avatar i')).toHaveCSS('display', 'none');
  await expect(page.locator('.dock-bubbles .dock-bubble')).toHaveCount(0);
});

/**
 * The In-Call frame counts up from the answer, which only the Rust registry can know:
 * it is built from the same call events the call log stores. The number badge moves to
 * the timer chip, because two counters on one icon say nothing.
 */
test('a live call puts the timer on the dock', async ({ page }) => {
  await page.goto('/?mock=1&scenario=active-call#/dock');
  await page.reload();

  const dock = page.locator('.floating-dock');
  await expect(dock).toHaveClass(/dock-mood-call/);
  await expect(dock.locator('.dock-state-chip.call')).toHaveText(/02:1[45]/);
  // The count badge has to step aside for the timer; the avatar is the icon's face, not a badge.
  await expect(dock.locator('.dock-badge')).toHaveCount(0);
  await expect(dock).toHaveAttribute('title', 'In call with Arjun Kumar');
});

test.describe('icon morph', () => {
  /**
   * Settings → "Icon morph (on activity)" and "Revert delay" are one feature: the newest
   * sender borrows the icon's face, then gives it back. The switch itself decides whether
   * the face is worn at all.
   */
  test('the switch decides whether a chat may wear the icon', async ({ page }) => {
    await page.goto('/?mock=1#/dock');
    await page.reload();
    await expect(page.locator('.floating-dock .avatar')).toHaveCount(1);

    await page.goto('/?mock=1&scenario=morph-off#/dock');
    await expect(page.locator('.floating-dock .avatar')).toHaveCount(0);
    await expect(page.locator('.floating-dock > svg')).toHaveCount(1);
    // The count is the other half of the affordance, and it survives the morph being off.
    await expect(page.locator('.floating-dock > .dock-badge')).toHaveCount(1);
  });

  /**
   * The counter used to be styled as `.floating-dock > span`, which also caught the morphed
   * avatar and pinned it to the corner as a rounded square. The board keeps the face round
   * and in the middle of the icon, with the badge overlapping one corner of it.
   */
  test('the borrowed face stays round and centred', async ({ page }) => {
    await page.goto('/?mock=1#/dock');
    await page.reload();

    const dock = page.locator('.floating-dock');
    const face = dock.locator('.avatar');
    await expect(face).toHaveCSS('border-radius', '50%');
    await expect(face).toHaveCSS('position', 'relative');

    const box = await dock.boundingBox();
    const faceBox = await face.boundingBox();
    expect(box && faceBox).toBeTruthy();
    expect(Math.abs(faceBox!.x + faceBox!.width / 2 - (box!.x + box!.width / 2))).toBeLessThan(3);
    expect(Math.abs(faceBox!.y + faceBox!.height / 2 - (box!.y + box!.height / 2))).toBeLessThan(3);
  });

  test('the borrowed face reverts on the stored delay', async ({ page }) => {
    await page.goto('/?mock=1&scenario=morph-fast#/dock');
    await expect(page.locator('.floating-dock .avatar')).toHaveCount(1);
    await page.waitForTimeout(1_400);
    await expect(page.locator('.floating-dock .avatar')).toHaveCount(0);
    await expect(page.locator('.floating-dock > svg')).toHaveCount(1);
  });
});

test.describe('reduce motion', () => {
  test.use({ reducedMotion: 'reduce' });

  /** The board loops the unread ring; the spec says to stop looping and just fade. */
  test('the unread ring does not pulse', async ({ page }) => {
    await page.goto('/?mock=1#/dock');
    await page.reload();

    const dock = page.locator('.floating-dock');
    await expect(dock).toHaveClass(/has-unread/);
    await expect(dock).toHaveCSS('animation-name', 'none');
  });
});
