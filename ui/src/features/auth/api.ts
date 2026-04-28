import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query';
import type { ApiError } from '@/shared/lib/http';
import type { LoginRequest, LoginResponse, MeResponse } from '@/shared/types';
import { useMutation, useQuery } from '@tanstack/react-query';
import { apiRequest } from '@/shared/lib/http';
import { queryKeys } from '@/shared/lib/query-keys';

export function useLoginMutation(options?: UseMutationOptions<LoginResponse, ApiError, LoginRequest>) {
  return useMutation<LoginResponse, ApiError, LoginRequest>({
    mutationFn: (body) =>
      apiRequest<LoginResponse>('/api/auth/login', {
        method: 'POST',
        body,
        noAuth: true,
      }),
    ...options,
  });
}

export function useMeQuery(opts?: Partial<UseQueryOptions<MeResponse, ApiError>>) {
  return useQuery<MeResponse, ApiError>({
    queryKey: queryKeys.me,
    queryFn: () => apiRequest<MeResponse>('/api/me'),
    ...opts,
  });
}
