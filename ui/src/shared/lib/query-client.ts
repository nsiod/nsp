import { QueryClient } from '@tanstack/react-query';

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: (failureCount, error) => {
          if (error && typeof error === 'object' && 'status' in error) {
            const s = (error as { status?: number }).status;
            if (s === 401 || s === 403 || s === 404 || s === 422)
              return false;
          }
          return failureCount < 2;
        },
        staleTime: 5_000,
        refetchOnWindowFocus: true,
      },
    },
  });
}
