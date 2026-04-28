import type { UseQueryOptions } from '@tanstack/react-query';
import type { ApiError } from '@/shared/lib/http';
import type { ServerSettings, ServerSettingsPatch } from '@/shared/types';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { apiRequest } from '@/shared/lib/http';
import { queryKeys } from '@/shared/lib/query-keys';

export function useSettingsQuery(opts?: Partial<UseQueryOptions<ServerSettings, ApiError>>) {
  return useQuery<ServerSettings, ApiError>({
    queryKey: queryKeys.settings,
    queryFn: () => apiRequest<ServerSettings>('/api/settings'),
    retry: false,
    ...opts,
  });
}

export function useUpdateSettingsMutation() {
  const qc = useQueryClient();
  return useMutation<ServerSettings, ApiError, ServerSettingsPatch>({
    mutationFn: (body) => apiRequest<ServerSettings>('/api/settings', { method: 'PATCH', body }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.settings });
      qc.invalidateQueries({ queryKey: queryKeys.ssStatus });
      qc.invalidateQueries({ queryKey: queryKeys.wgStatus });
      qc.invalidateQueries({ queryKey: queryKeys.status });
    },
  });
}

export function useReloadMutation() {
  const qc = useQueryClient();
  return useMutation<void, ApiError, void>({
    mutationFn: () => apiRequest<void>('/api/reload', { method: 'POST' }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.settings });
      qc.invalidateQueries({ queryKey: queryKeys.ssStatus });
      qc.invalidateQueries({ queryKey: queryKeys.wgStatus });
      qc.invalidateQueries({ queryKey: queryKeys.status });
    },
  });
}
