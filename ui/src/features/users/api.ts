// All user-scoped reads/writes (including per-user SS/WG detail + rotate)
// live here. See api.architecture.test.ts for the regression guard against
// resurrecting the old per-protocol endpoints.

import type { UseQueryOptions } from '@tanstack/react-query';
import type { ApiError } from '@/shared/lib/http';
import type {
  UserCreateRequest,
  UserEntry,
  UserProtocolAck,
  UserSsDetail,
  UserSsEnabled,
  UserUpdateRequest,
  UserWgEnabled,
  UserWgEnableRequest,
  WgPeer,
} from '@/shared/types';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { apiRequest } from '@/shared/lib/http';
import { queryKeys } from '@/shared/lib/query-keys';

export function useUsersQuery(opts?: Partial<UseQueryOptions<UserEntry[], ApiError>>) {
  return useQuery<UserEntry[], ApiError>({
    queryKey: queryKeys.users,
    queryFn: () => apiRequest<UserEntry[]>('/api/users'),
    ...opts,
  });
}

export function useUserQuery(id: string, opts?: Partial<UseQueryOptions<UserEntry, ApiError>>) {
  return useQuery<UserEntry, ApiError>({
    queryKey: queryKeys.user(id),
    queryFn: () => apiRequest<UserEntry>(`/api/users/${encodeURIComponent(id)}`),
    enabled: !!id,
    ...opts,
  });
}

export function useCreateUserMutation() {
  const qc = useQueryClient();
  return useMutation<UserEntry, ApiError, UserCreateRequest>({
    mutationFn: (body) => apiRequest<UserEntry>('/api/users', { method: 'POST', body }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.users });
    },
  });
}

export function useUpdateUserMutation() {
  const qc = useQueryClient();
  return useMutation<UserEntry, ApiError, { id: string; body: UserUpdateRequest }>({
    mutationFn: ({ id, body }) =>
      apiRequest<UserEntry>(`/api/users/${encodeURIComponent(id)}`, { method: 'PATCH', body }),
    onSuccess: (data) => {
      qc.invalidateQueries({ queryKey: queryKeys.users });
      qc.invalidateQueries({ queryKey: queryKeys.user(data.id) });
    },
  });
}

export function useDeleteUserMutation() {
  const qc = useQueryClient();
  return useMutation<void, ApiError, string>({
    mutationFn: (id) =>
      apiRequest<void>(`/api/users/${encodeURIComponent(id)}`, { method: 'DELETE' }),
    onSuccess: (_data, id) => {
      qc.invalidateQueries({ queryKey: queryKeys.users });
      qc.invalidateQueries({ queryKey: queryKeys.user(id) });
      qc.invalidateQueries({ queryKey: queryKeys.ssStatus });
      qc.invalidateQueries({ queryKey: queryKeys.wgStatus });
      qc.invalidateQueries({ queryKey: queryKeys.status });
    },
  });
}

// ---- per-user SS ----

export function useUserSsDetailQuery(
  id: string,
  opts?: Partial<UseQueryOptions<UserSsDetail, ApiError>>,
) {
  return useQuery<UserSsDetail, ApiError>({
    queryKey: queryKeys.userSs(id),
    queryFn: () => apiRequest<UserSsDetail>(`/api/users/${encodeURIComponent(id)}/ss`),
    enabled: !!id,
    ...opts,
  });
}

export function useEnableUserSsMutation() {
  const qc = useQueryClient();
  return useMutation<UserSsEnabled, ApiError, string>({
    mutationFn: (id) =>
      apiRequest<UserSsEnabled>(`/api/users/${encodeURIComponent(id)}/ss`, { method: 'POST' }),
    onSuccess: (_data, id) => {
      qc.invalidateQueries({ queryKey: queryKeys.users });
      qc.invalidateQueries({ queryKey: queryKeys.user(id) });
      qc.invalidateQueries({ queryKey: queryKeys.userSs(id) });
      qc.invalidateQueries({ queryKey: queryKeys.ssStatus });
      qc.invalidateQueries({ queryKey: queryKeys.status });
    },
  });
}

export function useDisableUserSsMutation() {
  const qc = useQueryClient();
  return useMutation<UserProtocolAck, ApiError, string>({
    mutationFn: (id) =>
      apiRequest<UserProtocolAck>(`/api/users/${encodeURIComponent(id)}/ss`, {
        method: 'DELETE',
      }),
    onSuccess: (_data, id) => {
      qc.invalidateQueries({ queryKey: queryKeys.users });
      qc.invalidateQueries({ queryKey: queryKeys.user(id) });
      qc.invalidateQueries({ queryKey: queryKeys.userSs(id) });
      qc.invalidateQueries({ queryKey: queryKeys.ssStatus });
      qc.invalidateQueries({ queryKey: queryKeys.status });
    },
  });
}

export function useRotateUserSsMutation() {
  const qc = useQueryClient();
  return useMutation<UserSsEnabled, ApiError, string>({
    mutationFn: (id) =>
      apiRequest<UserSsEnabled>(`/api/users/${encodeURIComponent(id)}/ss/rotate`, {
        method: 'POST',
      }),
    onSuccess: (_data, id) => {
      qc.invalidateQueries({ queryKey: queryKeys.userSs(id) });
    },
  });
}

// ---- per-user WG ----

export function useUserWgDetailQuery(id: string, opts?: Partial<UseQueryOptions<WgPeer, ApiError>>) {
  return useQuery<WgPeer, ApiError>({
    queryKey: queryKeys.userWg(id),
    queryFn: () => apiRequest<WgPeer>(`/api/users/${encodeURIComponent(id)}/wg`),
    enabled: !!id,
    ...opts,
  });
}

export function useEnableUserWgMutation() {
  const qc = useQueryClient();
  return useMutation<UserWgEnabled, ApiError, { id: string; body?: UserWgEnableRequest }>({
    mutationFn: ({ id, body }) =>
      apiRequest<UserWgEnabled>(`/api/users/${encodeURIComponent(id)}/wg`, {
        method: 'POST',
        body: body ?? {},
      }),
    onSuccess: (_data, { id }) => {
      qc.invalidateQueries({ queryKey: queryKeys.users });
      qc.invalidateQueries({ queryKey: queryKeys.user(id) });
      qc.invalidateQueries({ queryKey: queryKeys.userWg(id) });
      qc.invalidateQueries({ queryKey: queryKeys.wgStatus });
      qc.invalidateQueries({ queryKey: queryKeys.status });
    },
  });
}

export function useDisableUserWgMutation() {
  const qc = useQueryClient();
  return useMutation<UserProtocolAck, ApiError, string>({
    mutationFn: (id) =>
      apiRequest<UserProtocolAck>(`/api/users/${encodeURIComponent(id)}/wg`, {
        method: 'DELETE',
      }),
    onSuccess: (_data, id) => {
      qc.invalidateQueries({ queryKey: queryKeys.users });
      qc.invalidateQueries({ queryKey: queryKeys.user(id) });
      qc.invalidateQueries({ queryKey: queryKeys.userWg(id) });
      qc.invalidateQueries({ queryKey: queryKeys.wgStatus });
      qc.invalidateQueries({ queryKey: queryKeys.status });
    },
  });
}

export function useRotateUserWgMutation() {
  const qc = useQueryClient();
  return useMutation<UserWgEnabled, ApiError, { id: string; body?: UserWgEnableRequest }>({
    mutationFn: ({ id, body }) =>
      apiRequest<UserWgEnabled>(`/api/users/${encodeURIComponent(id)}/wg/rotate`, {
        method: 'POST',
        body: body ?? {},
      }),
    onSuccess: (_data, { id }) => {
      qc.invalidateQueries({ queryKey: queryKeys.userWg(id) });
    },
  });
}
