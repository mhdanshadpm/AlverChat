import i18next from 'i18next';
import { initReactI18next } from 'react-i18next';
export type Catalog = Record<string, string>;

/**
 * Every language this app can serve, and the only list the picker, the catalogs and i18next
 * are read against. `i18n.keys.test.ts` fails it against the catalog directories, so a
 * language here with no strings beside it is caught instead of silently falling back.
 */
export const LANGUAGES = ['en'];

const RTL_LANGUAGES = ['ar', 'he', 'fa', 'ur'];

let initialized = false;

/**
 * Screens ship their own English catalog file and register it here, so adding a
 * screen never means editing a shared string file.
 */
export function registerNamespace(namespace: string, catalog: Catalog): void {
  if (!initialized) init();
  for (const [key, value] of Object.entries(catalog)) {
    i18next.addResource('en', namespace, key, value);
  }
}

function init(): void {
  initialized = true;
  void i18next.use(initReactI18next).init({
    lng: LANGUAGES[0],
    fallbackLng: 'en',
    supportedLngs: LANGUAGES,
    resources: { en: {} },
    interpolation: { escapeValue: false },
    returnNull: false,
  });
}

export function t(key: string, fallback: string, vars?: Record<string, unknown>): string {
  if (!initialized) init();
  const value = i18next.t(key, { defaultValue: fallback, ...vars });
  return typeof value === 'string' ? value : fallback;
}

/** `const text = ns('settings')` then `text('theme', 'Theme')`. */
export function ns(namespace: string): (key: string, fallback: string, vars?: Record<string, unknown>) => string {
  return (key, fallback, vars) => t(`${namespace}:${key}`, fallback, vars);
}

export function language(): string {
  return i18next.language || 'en';
}

export function isRtl(lang = language()): boolean {
  return RTL_LANGUAGES.includes(lang.split('-')[0]);
}

/**
 * Switch the catalogs. Settings' `language` is the store — this is the only writer, and the
 * only reader is `useAppBridge`, so the pick and the strings cannot come apart.
 */
export function setLanguage(lang: string): void {
  if (!initialized) init();
  void i18next.changeLanguage(lang);
  applyDirection();
}

export function applyDirection(): void {
  if (typeof document === 'undefined') return;
  document.documentElement.dir = isRtl() ? 'rtl' : 'ltr';
  document.documentElement.lang = language();
}
