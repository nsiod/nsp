import type { ReactNode } from 'react';
import { createContext, use, useEffect, useMemo, useState } from 'react';

export type Theme = 'light' | 'dark' | 'system';

interface ThemeProviderState {
  theme: Theme;
  setTheme: (theme: Theme) => void;
}

const STORAGE_KEY = 'nsp.theme';

const ThemeContext = createContext<ThemeProviderState | null>(null);

function readStoredTheme(): Theme {
  if (typeof window === 'undefined')
    return 'system';
  const raw = window.localStorage.getItem(STORAGE_KEY);
  if (raw === 'light' || raw === 'dark' || raw === 'system')
    return raw;
  return 'system';
}

function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  root.classList.remove('light', 'dark');
  if (theme === 'system') {
    const mql = window.matchMedia('(prefers-color-scheme: dark)');
    root.classList.add(mql.matches ? 'dark' : 'light');
    return;
  }
  root.classList.add(theme);
}

interface ThemeProviderProps {
  children: ReactNode;
}

export function ThemeProvider({ children }: ThemeProviderProps) {
  // eslint-disable-next-line react/use-state -- exposed setter is `setTheme` (below)
  const [theme, setThemeState] = useState<Theme>(readStoredTheme);

  useEffect(() => {
    applyTheme(theme);
    if (theme !== 'system')
      return;
    const mql = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = () => applyTheme('system');
    mql.addEventListener('change', onChange);
    return () => mql.removeEventListener('change', onChange);
  }, [theme]);

  const value = useMemo<ThemeProviderState>(
    () => ({
      theme,
      setTheme: (next) => {
        window.localStorage.setItem(STORAGE_KEY, next);
        setThemeState(next);
      },
    }),
    [theme],
  );

  return <ThemeContext value={value}>{children}</ThemeContext>;
}

export function useTheme(): ThemeProviderState {
  const ctx = use(ThemeContext);
  if (!ctx)
    throw new Error('useTheme must be used inside <ThemeProvider>');
  return ctx;
}
