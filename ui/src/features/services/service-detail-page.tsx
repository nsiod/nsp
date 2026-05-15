import type { UserEntry } from '@/shared/types';
import { Link, useParams } from '@tanstack/react-router';
import { ArrowLeft, Play, Square } from 'lucide-react';
import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
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
import { useUsersQuery } from '@/features/users/api';
import { Badge } from '@/shared/components/ui/badge';
import { Button, buttonVariants } from '@/shared/components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/shared/components/ui/card';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/shared/components/ui/table';
import { useToaster } from '@/shared/components/ui/toast';
import { formatTimestamp } from '@/shared/lib/utils';

type ServiceId = 'shadowsocks' | 'wireguard' | 'proxy';

function isServiceId(value: string): value is ServiceId {
  return value === 'shadowsocks' || value === 'wireguard' || value === 'proxy';
}

export function ServiceDetailPage() {
  const { id } = useParams({ from: '/_auth/services/$id' });
  if (!isServiceId(id))
    return <ServiceNotFound />;

  if (id === 'shadowsocks')
    return <ShadowsocksDetail />;
  if (id === 'wireguard')
    return <WireguardDetail />;
  return <ProxyDetail />;
}

function ServiceNotFound() {
  const { t } = useTranslation();
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t('serviceDetail.notFoundTitle')}</CardTitle>
        <CardDescription>{t('serviceDetail.notFoundDescription')}</CardDescription>
      </CardHeader>
      <CardContent>
        <Link to="/services" className={buttonVariants({ variant: 'outline' })}>
          <ArrowLeft className="mr-2 h-4 w-4" />
          {t('serviceDetail.back')}
        </Link>
      </CardContent>
    </Card>
  );
}

function ShadowsocksDetail() {
  const { t } = useTranslation();
  const status = useSsStatusQuery();
  const start = useSsStartMutation();
  const stop = useSsStopMutation();
  const users = useUsersQuery();
  const ssUsers = useMemo<UserEntry[]>(
    () => (users.data ?? []).filter((u) => u.ss_enabled),
    [users.data],
  );
  const data = status.data;
  const driverDown = status.error?.isUnavailable() ?? false;

  const fields: Array<[string, string | number | undefined | null]> = [
    [t('serviceDetail.ss.publicHost'), data?.public_host],
    [t('serviceDetail.ss.listenPort'), data?.listen_port],
    [t('serviceDetail.ss.method'), data?.method],
    [t('serviceDetail.ss.users'), data?.users ?? ssUsers.length],
    [t('serviceDetail.ss.reloads'), data?.reload_count],
    [t('serviceDetail.ss.lastSwap'), data ? formatMs(data.last_swap_ms) : null],
  ];

  return (
    <ServiceDetailLayout
      name={t('serviceDetail.ss.title')}
      description={t('services.descriptions.shadowsocks')}
      running={data?.running ?? false}
      available={data?.available ?? true}
      reason={data?.reason ?? null}
      driverDown={driverDown}
      onStart={() => start.mutate(undefined)}
      onStop={() => stop.mutate(undefined)}
      starting={start.isPending}
      stopping={stop.isPending}
      fields={fields}
      users={ssUsers}
      usersLoading={users.isLoading}
    />
  );
}

function ProxyDetail() {
  const { t } = useTranslation();
  const status = useProxyStatusQuery();
  const start = useProxyStartMutation();
  const stop = useProxyStopMutation();
  const users = useUsersQuery();
  const proxyUsers = useMemo<UserEntry[]>(
    () => (users.data ?? []).filter((u) => u.proxy_enabled),
    [users.data],
  );
  const data = status.data;
  const driverDown = status.error?.isUnavailable() ?? false;

  const fields: Array<[string, string | number | undefined | null]> = [
    [t('serviceDetail.proxy.publicHost'), data?.public_host],
    [t('serviceDetail.proxy.socks5Port'), data?.socks5_port],
    [t('serviceDetail.proxy.httpPort'), data?.http_port],
    [t('serviceDetail.proxy.users'), data?.users ?? proxyUsers.length],
    [t('serviceDetail.proxy.reloads'), data?.reload_count],
    [t('serviceDetail.proxy.lastSwap'), data ? formatMs(data.last_swap_ms) : null],
  ];

  return (
    <ServiceDetailLayout
      name={t('serviceDetail.proxy.title')}
      description={t('services.descriptions.proxy')}
      running={data?.running ?? false}
      available={data?.available ?? true}
      reason={data?.reason ?? null}
      driverDown={driverDown}
      onStart={() => start.mutate(undefined)}
      onStop={() => stop.mutate(undefined)}
      starting={start.isPending}
      stopping={stop.isPending}
      fields={fields}
      users={proxyUsers}
      usersLoading={users.isLoading}
    />
  );
}

function WireguardDetail() {
  const { t } = useTranslation();
  const status = useWgStatusQuery();
  const start = useWgStartMutation();
  const stop = useWgStopMutation();
  const users = useUsersQuery();
  const wgUsers = useMemo<UserEntry[]>(
    () => (users.data ?? []).filter((u) => u.wg_enabled),
    [users.data],
  );
  const data = status.data;
  const driverDown = status.error?.isUnavailable() ?? false;

  const fields: Array<[string, string | number | undefined | null]> = [
    [t('serviceDetail.wg.interface'), data?.interface],
    [t('serviceDetail.wg.listenPort'), data?.listen_port],
    [t('serviceDetail.wg.subnet'), data?.subnet],
    [t('serviceDetail.wg.endpoint'), data?.endpoint_host],
    [t('serviceDetail.wg.serverPublicKey'), data?.server_public_key],
    [t('serviceDetail.wg.totalPeers'), data?.total_peers ?? wgUsers.length],
  ];

  return (
    <ServiceDetailLayout
      name={t('serviceDetail.wg.title')}
      description={t('services.descriptions.wireguard')}
      running={data?.running ?? false}
      available={data?.available ?? true}
      reason={data?.reason ?? null}
      driverDown={driverDown}
      onStart={() => start.mutate(undefined)}
      onStop={() => stop.mutate(undefined)}
      starting={start.isPending}
      stopping={stop.isPending}
      fields={fields}
      users={wgUsers}
      usersLoading={users.isLoading}
    />
  );
}

interface ServiceDetailLayoutProps {
  name: string;
  description: string;
  running: boolean;
  available: boolean;
  reason: string | null;
  driverDown: boolean;
  onStart: () => void;
  onStop: () => void;
  starting: boolean;
  stopping: boolean;
  fields: Array<[string, string | number | undefined | null]>;
  users: UserEntry[];
  usersLoading: boolean;
}

function ServiceDetailLayout({
  name,
  description,
  running,
  available,
  reason,
  driverDown,
  onStart,
  onStop,
  starting,
  stopping,
  fields,
  users,
  usersLoading,
}: ServiceDetailLayoutProps) {
  const { t } = useTranslation();
  const toaster = useToaster();
  const busy = starting || stopping;

  const handleStart = () => {
    if (driverDown || running || !available)
      return;
    try {
      onStart();
    }
    catch (err) {
      toaster.error(t('services.startFailed', { name }), (err as Error).message);
    }
  };
  const handleStop = () => {
    if (driverDown || !running)
      return;
    try {
      onStop();
    }
    catch (err) {
      toaster.error(t('services.stopFailed', { name }), (err as Error).message);
    }
  };

  return (
    <div className="grid gap-4">
      <div>
        <Link
          to="/services"
          className="inline-flex items-center text-xs text-muted-foreground hover:underline"
        >
          <ArrowLeft className="mr-1 h-3.5 w-3.5" />
          {t('serviceDetail.back')}
        </Link>
        <div className="mt-1 flex flex-wrap items-center gap-2">
          <h1 className="text-xl font-semibold tracking-tight">{name}</h1>
          {running
            ? <Badge variant="success">{t('services.running')}</Badge>
            : <Badge variant="muted">{t('services.stopped')}</Badge>}
          {!available && !running
            ? <Badge variant="destructive">{t('serviceDetail.unavailable')}</Badge>
            : null}
        </div>
        <p className="mt-1 text-sm text-muted-foreground">{description}</p>
      </div>

      {driverDown
        ? (
            <div className="rounded-md border border-amber-300 bg-amber-50 p-3 text-xs text-amber-900 dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-200">
              {t('services.driverDown')}
            </div>
          )
        : null}

      {!running && !available && reason
        ? (
            <div className="text-xs text-muted-foreground">
              {t('services.preconditions')}
              {' '}
              <span className="font-mono">{reason}</span>
            </div>
          )
        : null}

      <Card>
        <CardHeader>
          <CardTitle className="text-base">{t('serviceDetail.statusHeading')}</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-2 text-sm">
          <dl className="grid gap-1">
            {fields.map(([k, v]) => (
              <div key={k} className="grid grid-cols-[160px_1fr] items-baseline gap-2">
                <dt className="text-xs uppercase tracking-wide text-muted-foreground">{k}</dt>
                <dd className="break-all font-mono text-xs text-foreground/90">
                  {v === undefined || v === null || v === '' ? '—' : String(v)}
                </dd>
              </div>
            ))}
          </dl>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            {t('serviceDetail.usersHeading', { count: users.length })}
          </CardTitle>
          <CardDescription>{t('serviceDetail.usersDescription')}</CardDescription>
        </CardHeader>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t('users.table.name')}</TableHead>
                <TableHead>{t('users.table.created')}</TableHead>
                <TableHead className="text-right">{t('users.table.actions')}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {usersLoading
                ? (
                    <TableRow>
                      <TableCell colSpan={3} className="text-center text-muted-foreground">
                        {t('common.loading')}
                      </TableCell>
                    </TableRow>
                  )
                : users.length === 0
                  ? (
                      <TableRow>
                        <TableCell colSpan={3} className="text-center text-muted-foreground">
                          {t('serviceDetail.usersEmpty')}
                        </TableCell>
                      </TableRow>
                    )
                  : (
                      users.map((u) => (
                        <TableRow key={u.id}>
                          <TableCell>
                            <Link
                              to="/users/$name"
                              params={{ name: u.name }}
                              className="hover:underline"
                            >
                              {u.name}
                            </Link>
                          </TableCell>
                          <TableCell className="text-xs text-muted-foreground">
                            {formatTimestamp(u.created_at)}
                          </TableCell>
                          <TableCell className="text-right">
                            <Link
                              to="/users/$name"
                              params={{ name: u.name }}
                              aria-label={t('users.openAria', { name: u.name })}
                              className={buttonVariants({ variant: 'ghost', size: 'sm' })}
                            >
                              {t('users.detail')}
                            </Link>
                          </TableCell>
                        </TableRow>
                      ))
                    )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <div className="flex items-center justify-end gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={handleStart}
          disabled={driverDown || busy || running || !available}
          aria-label={t('services.startAria', { name })}
        >
          <Play className="mr-2 h-4 w-4" />
          {t('services.start')}
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={handleStop}
          disabled={driverDown || busy || !running}
          aria-label={t('services.stopAria', { name })}
        >
          <Square className="mr-2 h-4 w-4" />
          {t('services.stop')}
        </Button>
      </div>
    </div>
  );
}

function formatMs(ms: number): string {
  if (ms <= 0)
    return '—';
  if (ms < 1000)
    return `${ms} ms`;
  return `${(ms / 1000).toFixed(2)} s`;
}

export default ServiceDetailPage;
