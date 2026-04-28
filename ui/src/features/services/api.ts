// Top-level health (`useStatusQuery`) plus protocol service lifecycle (status,
// start, stop) for both Shadowsocks and WireGuard. Per-user SS / WG
// hooks live in `features/users/api.ts`.

import type { UseQueryOptions } from '@tanstack/react-query';
import type { ApiError } from '@/shared/lib/http';
import type { SsStatus, StatusResponse, WgStatus } from '@/shared/types';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { apiRequest } from '@/shared/lib/http';
import { queryKeys } from '@/shared/lib/query-keys';

export function useStatusQuery(opts?: Partial<UseQueryOptions<StatusResponse, ApiError>>) {
  return useQuery<StatusResponse, ApiError>({
    queryKey: queryKeys.status,
    queryFn: () => apiRequest<StatusResponse>('/api/status'),
    refetchInterval: 30_000,
    ...opts,
  });
}

export function useSsStatusQuery(opts?: Partial<UseQueryOptions<SsStatus, ApiError>>) {
  return useQuery<SsStatus, ApiError>({
    queryKey: queryKeys.ssStatus,
    queryFn: () => apiRequest<SsStatus>('/api/protocol/ss/status'),
    refetchInterval: 15_000,
    ...opts,
  });
}

export function useSsStartMutation() {
  const qc = useQueryClient();
  return useMutation<void, ApiError, void>({
    mutationFn: () => apiRequest<void>('/api/protocol/ss/start', { method: 'POST' }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.ssStatus });
      qc.invalidateQueries({ queryKey: queryKeys.status });
    },
  });
}

export function useSsStopMutation() {
  const qc = useQueryClient();
  return useMutation<void, ApiError, void>({
    mutationFn: () => apiRequest<void>('/api/protocol/ss/stop', { method: 'POST' }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.ssStatus });
      qc.invalidateQueries({ queryKey: queryKeys.status });
    },
  });
}

export function useWgStatusQuery(opts?: Partial<UseQueryOptions<WgStatus, ApiError>>) {
  return useQuery<WgStatus, ApiError>({
    queryKey: queryKeys.wgStatus,
    queryFn: () => apiRequest<WgStatus>('/api/protocol/wg/status'),
    refetchInterval: 15_000,
    ...opts,
  });
}

export function useWgStartMutation() {
  const qc = useQueryClient();
  return useMutation<void, ApiError, void>({
    mutationFn: () => apiRequest<void>('/api/protocol/wg/start', { method: 'POST' }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.wgStatus });
      qc.invalidateQueries({ queryKey: queryKeys.status });
    },
  });
}

export function useWgStopMutation() {
  const qc = useQueryClient();
  return useMutation<void, ApiError, void>({
    mutationFn: () => apiRequest<void>('/api/protocol/wg/stop', { method: 'POST' }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.wgStatus });
      qc.invalidateQueries({ queryKey: queryKeys.status });
    },
  });
}
