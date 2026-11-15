import { describe, expect, it } from 'vitest';
import chat from './en/chat';
import groups from './en/groups';
import onboarding from './en/onboarding';
import people from './en/people';
import profile from './en/profile';
import settings from './en/settings';
import shell from './en/shell';
import system from './en/system';

const CATALOGS = { chat, groups, onboarding, people, profile, settings, shell, system };

describe('English catalogs', () => {
  it('holds flat string keys, because i18next splits dots into namespaces', () => {
    for (const [name, catalog] of Object.entries(CATALOGS)) {
      for (const [key, value] of Object.entries(catalog)) {
        expect(typeof value, `${name}.${key}`).toBe('string');
        expect(key, `${name}.${key}`).not.toContain('.');
        expect(value.length, `${name}.${key}`).toBeGreaterThan(0);
      }
    }
  });
});
