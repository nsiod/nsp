import type { UseQueryOptions } from '@tanstack/react-query';
import type { ApiError } from '@/shared/lib/http';
import type {
  IptablesCreateRequest,
  IptablesReconcileReport,
  IptablesRule,
  IptablesSource,
  IptablesVerifyRequest,
} from '@/shared/types';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { apiRequest } from '@/shared/lib/http';
import { queryKeys } from '@/shared/lib/query-keys';

export function useIptablesRulesQuery(
  source?: IptablesSource,
  opts?: Partial<UseQueryOptions<IptablesRule[], ApiError>>,
) {
  return useQuery<IptablesRule[], ApiError>({
    queryKey: queryKeys.iptables(source),
    queryFn: () =>
      apiRequest<IptablesRule[]>('/api/iptables', {
        query: source ? { source } : undefined,
      }),
    ...opts,
  });
}

export function useCreateIptablesRuleMutation() {
  const qc = useQueryClient();
  return useMutation<IptablesRule, ApiError, IptablesCreateRequest>({
    mutationFn: (body) => apiRequest<IptablesRule>('/api/iptables', { method: 'POST', body }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['iptables'] });
    },
  });
}

export function useDeleteIptablesRuleMutation() {
  const qc = useQueryClient();
  return useMutation<void, ApiError, string>({
    mutationFn: (id) =>
      apiRequest<void>(`/api/iptables/${encodeURIComponent(id)}`, { method: 'DELETE' }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['iptables'] });
    },
  });
}

export function useVerifyIptablesRuleMutation() {
  return useMutation<{ ok: boolean }, ApiError, IptablesVerifyRequest>({
    mutationFn: (body) =>
      apiRequest<{ ok: boolean }>('/api/iptables/verify', { method: 'POST', body }),
  });
}

export function useReconcileIptablesRulesMutation() {
  const qc = useQueryClient();
  return useMutation<IptablesReconcileReport, ApiError, void>({
    mutationFn: () =>
      apiRequest<IptablesReconcileReport>('/api/iptables/reconcile', { method: 'POST' }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['iptables'] });
    },
  });
}
