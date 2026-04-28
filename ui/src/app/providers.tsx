import type { ReactNode } from 'react';
import { QueryClientProvider } from '@tanstack/react-query';
import { useState } from 'react';
import { I18nextProvider } from 'react-i18next';
import i18n from '@/app/i18n';
import { ThemeProvider } from '@/shared/components/theme-provider';
import { ToasterProvider } from '@/shared/components/ui/toast';
import { createQueryClient } from '@/shared/lib/query-client';

interface ProvidersProps {
  children: ReactNode;
}

export function Providers({ children }: ProvidersProps) {
  const [queryClient] = useState(createQueryClient);

  return (
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <ThemeProvider>
          <ToasterProvider>{children}</ToasterProvider>
        </ThemeProvider>
      </QueryClientProvider>
    </I18nextProvider>
  );
}
