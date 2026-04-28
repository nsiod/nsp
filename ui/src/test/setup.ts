import '@testing-library/jest-dom/vitest';

// jsdom doesn't implement matchMedia. ThemeProvider and sonner both
// query it for `prefers-color-scheme`; stub to a no-op listener so
// rendering doesn't crash inside tests.
if (typeof window !== 'undefined' && typeof window.matchMedia !== 'function') {
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: (query: string): MediaQueryList => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    } as unknown as MediaQueryList),
  });
}
