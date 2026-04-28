import type { UseQueryOptions } from '@tanstack/react-query';
import type { ApiError } from '@/shared/lib/http';
import type { AuditEntry } from '@/shared/types';
import { useQuery } from '@tanstack/react-query';
import { apiRequest } from '@/shared/lib/http';
import { queryKeys } from '@/shared/lib/query-keys';

export function useAuditQuery(opts?: Partial<UseQueryOptions<AuditEntry[], ApiError>>) {
  return useQuery<AuditEntry[], ApiError>({
    queryKey: queryKeys.audit,
    queryFn: () => apiRequest<AuditEntry[]>('/api/audit'),
    retry: false,
    ...opts,
  });
}
