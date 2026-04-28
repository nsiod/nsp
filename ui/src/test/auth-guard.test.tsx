import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { createMemoryHistory, createRouter, RouterProvider } from '@tanstack/react-router';
import { render, screen } from '@testing-library/react';
import i18next from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { en } from '@/app/locales/en';
import { zhCN } from '@/app/locales/zh-CN';
import { routeTree } from '@/app/routeTree.gen';
import { ThemeProvider } from '@/shared/components/theme-provider';
import { ToasterProvider } from '@/shared/components/ui/toast';
import { authStore } from '@/shared/stores/auth';

beforeAll(async () => {
  if (!i18next.isInitialized) {
    await i18next.use(initReactI18next).init({
      lng: 'en',
      fallbackLng: 'en',
      resources: {
        'en': { translation: en },
        'zh-CN': { translation: zhCN },
      },
      interpolation: { escapeValue: false },
      returnNull: false,
    });
  }
  else {
    await i18next.changeLanguage('en');
  }
});

afterEach(() => {
  authStore.clear();
});

interface RenderOptions {
  initialPath: string;
}

async function renderApp({ initialPath }: RenderOptions) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const router = createRouter({
    routeTree,
    history: createMemoryHistory({ initialEntries: [initialPath] }),
    defaultPendingMs: 0,
  });
  const view = render(
    <I18nextProvider i18n={i18next}>
      <QueryClientProvider client={queryClient}>
        <ThemeProvider>
          <ToasterProvider>
            <RouterProvider router={router} />
          </ToasterProvider>
        </ThemeProvider>
      </QueryClientProvider>
    </I18nextProvider>,
  );
  await router.load();
  return { view, router };
}

describe('auth guard', () => {
  it('redirects unauthenticated visits to /users to /login', async () => {
    const { router } = await renderApp({ initialPath: '/users' });
    expect(router.state.location.pathname).toBe('/login');
    expect(await screen.findByText(en.login.title)).toBeInTheDocument();
  });

  it('lets authenticated visits reach /users', async () => {
    authStore.set('test-token', Math.floor(Date.now() / 1000) + 3600);
    const { router } = await renderApp({ initialPath: '/users' });
    expect(router.state.location.pathname).toBe('/users');
  });
});
