import { create } from 'zustand';
import { createJSONStorage, persist } from 'zustand/middleware';

interface AuthState {
  token: string | null;
  expiresAt: number | null;
  setSession: (token: string, expiresAt: number) => void;
  clear: () => void;
}

// JWT lives in `sessionStorage` so it is scoped to the tab and cleared on
// browser close. We deliberately avoid cookies (no CSRF surface) and
// `localStorage` (XSS-persisted token).
export const useAuthStore = create<AuthState>()(
  persist(
    (set) => ({
      token: null,
      expiresAt: null,
      setSession: (token, expiresAt) => set({ token, expiresAt }),
      clear: () => set({ token: null, expiresAt: null }),
    }),
    {
      name: 'nsp.auth',
      storage: createJSONStorage(() => sessionStorage),
    },
  ),
);

function isSessionLive(token: string | null, expiresAt: number | null): boolean {
  if (!token)
    return false;
  if (expiresAt != null && expiresAt * 1000 < Date.now())
    return false;
  return true;
}

// Imperative facade for non-React callers (HTTP client, click handlers,
// command-style code paths).
export const authStore = {
  getToken(): string | null {
    return useAuthStore.getState().token;
  },
  getExpiresAt(): number | null {
    return useAuthStore.getState().expiresAt;
  },
  isAuthenticated(): boolean {
    const { token, expiresAt } = useAuthStore.getState();
    return isSessionLive(token, expiresAt);
  },
  set(token: string, expiresAt: number): void {
    useAuthStore.getState().setSession(token, expiresAt);
  },
  clear(): void {
    useAuthStore.getState().clear();
  },
};

export { isSessionLive };
