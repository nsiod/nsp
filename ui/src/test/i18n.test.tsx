import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from '@tanstack/react-router';
import { render, screen } from '@testing-library/react';
import i18next from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { beforeAll, describe, expect, it } from 'vitest';
import { en } from '@/app/locales/en';
import { zhCN } from '@/app/locales/zh-CN';
import { LoginPage } from '@/features/auth/login-page';
import { ThemeProvider } from '@/shared/components/theme-provider';
import { ToasterProvider } from '@/shared/components/ui/toast';

beforeAll(async () => {
  if (!i18next.isInitialized) {
    await i18next.use(initReactI18next).init({
      lng: 'zh-CN',
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
    await i18next.changeLanguage('zh-CN');
  }
});

function renderLogin() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });

  const rootRoute = createRootRoute({ component: () => <Outlet /> });
  const loginRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/login',
    component: LoginPage,
  });
  const router = createRouter({
    routeTree: rootRoute.addChildren([loginRoute]),
    history: createMemoryHistory({ initialEntries: ['/login'] }),
  });

  return render(
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
}

describe('i18n', () => {
  it('renders Simplified Chinese copy on LoginPage when locale is zh-CN', async () => {
    renderLogin();
    expect(await screen.findByText(zhCN.login.title)).toBeInTheDocument();
    expect(screen.getByLabelText(new RegExp(zhCN.login.passwordLabel))).toBeInTheDocument();
    expect(screen.getByRole('button', { name: zhCN.login.submit })).toBeInTheDocument();
  });
});
