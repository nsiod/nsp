// i18next bootstrap for the nsp UI.
//
// Resources are bundled inline (see ./locales/*). Language selection is
// persisted in localStorage under `nsp.locale`; on first load we fall
// back to the browser language — any `zh*` tag activates `zh-CN`, anything
// else falls back to `en`.

import i18n from 'i18next';
import LanguageDetector from 'i18next-browser-languagedetector';
import { initReactI18next } from 'react-i18next';
import { en } from './locales/en';
import { zhCN } from './locales/zh-CN';

export const LOCALE_STORAGE_KEY = 'nsp.locale';

export const supportedLocales = ['en', 'zh-CN'] as const;
export type SupportedLocale = (typeof supportedLocales)[number];

function normalizeLocale(tag: string | null | undefined): SupportedLocale {
  if (!tag)
    return 'en';
  const lower = tag.toLowerCase();
  if (lower.startsWith('zh'))
    return 'zh-CN';
  return 'en';
}

void i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      'en': { translation: en },
      'zh-CN': { translation: zhCN },
    },
    fallbackLng: 'en',
    supportedLngs: [...supportedLocales],
    interpolation: { escapeValue: false },
    detection: {
      order: ['localStorage', 'navigator'],
      caches: ['localStorage'],
      lookupLocalStorage: LOCALE_STORAGE_KEY,
      // Coerce raw detected tags (e.g. `en-US@POSIX`, `zh-Hant-TW`) down to
      // one of our supported locales before i18next stores or persists it.
      // Without this, an unsupported locale ends up cached in localStorage
      // and changeLanguage(...) appears to "do nothing" because the active
      // language is never one of `en` / `zh-CN`.
      convertDetectedLanguage: (lng) => normalizeLocale(lng),
    },
    returnNull: false,
  });

if (typeof window !== 'undefined') {
  // Reflect the active language on <html lang> so screen readers and
  // user-agent stylesheets pick up the change. Also handy in DevTools.
  const apply = (lng: string) => {
    document.documentElement.lang = lng;
  };
  apply(i18n.resolvedLanguage ?? i18n.language ?? 'en');
  i18n.on('languageChanged', apply);
}

export default i18n;
