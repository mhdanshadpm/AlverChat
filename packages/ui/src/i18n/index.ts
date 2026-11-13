import i18next from 'i18next';
import { initReactI18next } from 'react-i18next';
import common from './en/common';

export type Catalog = Record<string, string>;

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
    lng: storedLanguage(),
    fallbackLng: 'en',
    supportedLngs: ['en'],
    ns: ['common'],
    defaultNS: 'common',
    resources: { en: { common } },
    interpolation: { escapeValue: false },
    returnNull: false,
  });
}

function storedLanguage(): string {
  try {
    return localStorage.getItem('alverchat.language') ?? 'en';
  } catch {
    return 'en';
  }
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

export function setLanguage(lang: string): void {
  if (!initialized) init();
  try {
    localStorage.setItem('alverchat.language', lang);
  } catch {
    // Private mode: the language only lives for this session.
  }
  void i18next.changeLanguage(lang);
  applyDirection();
}

export function applyDirection(): void {
  if (typeof document === 'undefined') return;
  document.documentElement.dir = isRtl() ? 'rtl' : 'ltr';
  document.documentElement.lang = language();
}

export { useTranslation } from 'react-i18next';
