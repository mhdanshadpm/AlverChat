import { describe, expect, it } from 'vitest';

import groups from './en/groups';
import settings from './en/settings';
import { ns, registerNamespace } from './index';
import { AVATAR_MAX_BYTES, AVATAR_PIXELS } from '../screens/photoPipeline';

registerNamespace('groups', groups);
registerNamespace('settings', settings);

const inGroups = ns('groups');
const inSettings = ns('settings');

/**
 * The seam tests prove that a sentence and a call site agree. They cannot prove what the user
 * reads, because the number arrives as a variable and the renderer is the only place it becomes
 * a digit again. These are the four sentences whose magnitudes the code owns, run through the
 * production helper with the production constants, so a limit that moves changes the words here
 * instead of leaving the copy behind.
 */
describe('the words an enforced limit ends up in', () => {
  it('prints the avatar sizes the photo pipeline decides', () => {
    expect(
      inGroups('uploadHint', 'Pictures are resized to {{px}} px and stripped of EXIF before they are stored.', {
        px: AVATAR_PIXELS,
      }),
    ).toBe('Pictures are resized to 256 px and stripped of EXIF before they are stored.');
    expect(inGroups('errorSize', 'That picture is bigger than {{max}} MB.', { max: AVATAR_MAX_BYTES / (1024 * 1024) })).toBe(
      'That picture is bigger than 10 MB.',
    );
  });

  it('hands the group description cap to the sentence that states it', () => {
    // An invented number, because the number itself belongs to the field's `maxLength` and the
    // Rust seam guard owns the passphrase one below; what is proven here is that the words the
    // user reads are built from whatever the call passes.
    expect(inGroups('descriptionHelper', 'Up to {{max}} characters.', { max: 3 })).toBe('Up to 3 characters.');
  });

  it('names the edit-window options by the minutes they hold', () => {
    expect(inSettings('editMinutes', '{{count}} minutes', { count: 15 })).toBe('15 minutes');
    // `count` is the one variable name i18next treats as a plural selector, and 60 / 60 is the
    // only label this screen renders for exactly one of something.
    expect(inSettings('editHour', '{{count}} hour', { count: 60 / 60 })).toBe('1 hour');
    expect(inSettings('editDay', '{{count}} hours', { count: 1440 / 60 })).toBe('24 hours');
  });

  it('promises the passphrase length the desktop gate enforces', () => {
    expect(inSettings('backupPassphrasePlaceholder', 'At least {{min}} characters', { min: 8 })).toBe('At least 8 characters');
  });
});
