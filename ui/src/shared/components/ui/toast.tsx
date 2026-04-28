// Toaster wrapper backed by sonner. The `useToaster()` hook keeps the
// imperative `toast / success / error / info` shape that call sites
// across the app use, while sonner handles rendering, stacking,
// animation, and accessibility.

import type { ReactNode } from 'react';
import { useMemo } from 'react';
import { toast as sonnerToast, Toaster as SonnerToaster } from 'sonner';
import { useTheme } from '@/shared/components/theme-provider';

interface ToastInput {
  title: string;
  description?: string;
  variant?: 'default' | 'destructive' | 'success';
}

interface ToasterContextValue {
  toast: (msg: ToastInput) => void;
  success: (title: string, description?: string) => void;
  error: (title: string, description?: string) => void;
  info: (title: string, description?: string) => void;
}

function callByVariant({ title, description, variant }: ToastInput): void {
  const opts = description ? { description } : undefined;
  if (variant === 'destructive') {
    sonnerToast.error(title, opts);
    return;
  }
  if (variant === 'success') {
    sonnerToast.success(title, opts);
    return;
  }
  sonnerToast(title, opts);
}

export function ToasterProvider({ children }: { children: ReactNode }) {
  const { theme } = useTheme();
  // sonner accepts 'light' | 'dark' | 'system'; ThemeProvider stores
  // the same set so we can pass it through directly.
  return (
    <>
      {children}
      <SonnerToaster
        theme={theme}
        position="bottom-right"
        richColors
        closeButton
        toastOptions={{
          classNames: {
            toast:
              'pointer-events-auto rounded-md border border-border bg-card text-card-foreground shadow-lg',
            title: 'text-sm font-semibold',
            description: 'text-sm text-muted-foreground',
          },
        }}
      />
    </>
  );
}

export function useToaster(): ToasterContextValue {
  return useMemo<ToasterContextValue>(
    () => ({
      toast: callByVariant,
      success: (title, description) =>
        callByVariant({ title, description, variant: 'success' }),
      error: (title, description) =>
        callByVariant({ title, description, variant: 'destructive' }),
      info: (title, description) =>
        callByVariant({ title, description, variant: 'default' }),
    }),
    [],
  );
}
