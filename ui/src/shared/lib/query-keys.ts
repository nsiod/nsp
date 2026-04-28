import type { IptablesSource } from '@/shared/types';

export const queryKeys = {
  status: ['status'] as const,
  me: ['me'] as const,
  users: ['users'] as const,
  user: (id: string) => ['users', id] as const,
  userSs: (id: string) => ['users', id, 'ss'] as const,
  userWg: (id: string) => ['users', id, 'wg'] as const,
  ssStatus: ['protocol', 'ss', 'status'] as const,
  wgStatus: ['protocol', 'wg', 'status'] as const,
  settings: ['settings'] as const,
  audit: ['audit'] as const,
  iptables: (source?: IptablesSource) =>
    source ? (['iptables', source] as const) : (['iptables'] as const),
};
