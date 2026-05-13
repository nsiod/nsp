import type { UseMutationResult } from '@tanstack/react-query';
import type { ApiError } from '@/shared/lib/http';
import type { ProxyStatus, SsStatus, WgStatus } from '@/shared/types';
import {
  useProxyStartMutation,
  useProxyStatusQuery,
  useProxyStopMutation,
  useSsStartMutation,
  useSsStatusQuery,
  useSsStopMutation,
  useWgStartMutation,
  useWgStatusQuery,
  useWgStopMutation,
} from '@/features/services/api';

export interface ServiceMetric {
  label: string;
  value: string;
}

export interface ServiceStatusCommon {
  running: boolean;
  available: boolean;
  reason?: string | null;
  metrics: ServiceMetric[];
  subtitle?: string;
}

export interface ServiceStatusQuery {
  data?: ServiceStatusCommon;
  isLoading: boolean;
  error: ApiError | null;
}

export interface ServiceDescriptor {
  id: 'shadowsocks' | 'wireguard' | 'proxy';
  name: string;
  description: string;
  useStatusQuery: () => ServiceStatusQuery;
  useStart: () => UseMutationResult<void, ApiError, void>;
  useStop: () => UseMutationResult<void, ApiError, void>;
}

function formatMs(ms: number): string {
  if (ms <= 0)
    return '—';
  if (ms < 1000)
    return `${ms} ms`;
  return `${(ms / 1000).toFixed(2)} s`;
}

function joinSubtitle(parts: Array<string | number | null | undefined>): string | undefined {
  const cleaned = parts
    .map((p) => (p === null || p === undefined ? '' : String(p).trim()))
    .filter((s) => s.length > 0 && s !== '0');
  return cleaned.length > 0 ? cleaned.join(' · ') : undefined;
}

function ssStatusToCommon(s: SsStatus): ServiceStatusCommon {
  return {
    running: s.running,
    available: s.available,
    reason: s.reason,
    subtitle: joinSubtitle([
      s.public_host && s.listen_port ? `${s.public_host}:${s.listen_port}` : '',
      s.method,
    ]),
    // Hide the metrics grid until the service is actually running —
    // otherwise paused / stopped cards show a noisy "0 / 0 / —" row.
    metrics: s.running
      ? [
          { label: 'Users', value: String(s.users) },
          { label: 'Reloads', value: String(s.reload_count) },
          { label: 'Last swap', value: formatMs(s.last_swap_ms) },
        ]
      : [],
  };
}

function wgStatusToCommon(s: WgStatus): ServiceStatusCommon {
  return {
    running: s.running,
    available: s.available,
    reason: s.reason,
    subtitle: joinSubtitle([s.interface, s.subnet]),
    metrics: s.running
      ? [
          { label: 'Peers', value: String(s.total_peers) },
          { label: 'Listen port', value: String(s.listen_port) },
          { label: 'Endpoint', value: s.endpoint_host ?? '—' },
        ]
      : [],
  };
}

function proxyStatusToCommon(s: ProxyStatus): ServiceStatusCommon {
  return {
    running: s.running,
    available: s.available,
    reason: s.reason,
    subtitle: joinSubtitle([
      s.public_host && s.socks5_port ? `socks5 ${s.public_host}:${s.socks5_port}` : '',
      s.public_host && s.http_port ? `http ${s.public_host}:${s.http_port}` : '',
    ]),
    metrics: s.running
      ? [
          { label: 'Users', value: String(s.users) },
          { label: 'SOCKS5 port', value: String(s.socks5_port) },
          { label: 'HTTP port', value: String(s.http_port) },
          { label: 'Reloads', value: String(s.reload_count) },
        ]
      : [],
  };
}

export const serviceDescriptors: ServiceDescriptor[] = [
  {
    id: 'shadowsocks',
    name: 'Shadowsocks',
    description: 'Embedded AEAD-2022 server. Runtime lifecycle is independent of config.',
    useStatusQuery: () => {
      const q = useSsStatusQuery();
      return {
        data: q.data ? ssStatusToCommon(q.data) : undefined,
        isLoading: q.isLoading,
        error: q.error ?? null,
      };
    },
    useStart: useSsStartMutation,
    useStop: useSsStopMutation,
  },
  {
    id: 'wireguard',
    name: 'WireGuard',
    description: 'Userspace WireGuard device. Requires CAP_NET_ADMIN and /dev/net/tun.',
    useStatusQuery: () => {
      const q = useWgStatusQuery();
      return {
        data: q.data ? wgStatusToCommon(q.data) : undefined,
        isLoading: q.isLoading,
        error: q.error ?? null,
      };
    },
    useStart: useWgStartMutation,
    useStop: useWgStopMutation,
  },
  {
    id: 'proxy',
    name: 'Proxy',
    description: 'SOCKS5 + HTTP CONNECT on independent ports, per-user auth.',
    useStatusQuery: () => {
      const q = useProxyStatusQuery();
      return {
        data: q.data ? proxyStatusToCommon(q.data) : undefined,
        isLoading: q.isLoading,
        error: q.error ?? null,
      };
    },
    useStart: useProxyStartMutation,
    useStop: useProxyStopMutation,
  },
];
