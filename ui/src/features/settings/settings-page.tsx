// Settings page. Backed by `/api/settings` + `/api/reload`. Password
// rotation invalidates every session (the server bumps `token_generation`)
// so the UI signs the operator out and redirects to /login on success.

import type { FormEvent } from 'react';
import type { ApiError } from '@/shared/lib/http';
import type { ServerSettingsPatch, WgSubnetConflictBody } from '@/shared/types';
import { useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { RefreshCw, Save } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { Trans, useTranslation } from 'react-i18next';
import { useSsStatusQuery, useWgStatusQuery } from '@/features/services/api';
import { useReloadMutation, useSettingsQuery, useUpdateSettingsMutation } from '@/features/settings/api';
import { Button } from '@/shared/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/components/ui/card';
import { Input } from '@/shared/components/ui/input';
import { Label } from '@/shared/components/ui/label';
import { Separator } from '@/shared/components/ui/separator';
import { useToaster } from '@/shared/components/ui/toast';
import { authStore } from '@/shared/stores/auth';

function parseSubnetConflict(err: ApiError): string[] | null {
  if (err.status !== 409 || !err.body)
    return null;
  try {
    const parsed = JSON.parse(err.body) as Partial<WgSubnetConflictBody>;
    if (parsed && parsed.code === 'wg-subnet-conflict' && Array.isArray(parsed.conflicts)) {
      return parsed.conflicts;
    }
  }
  catch {
    // fall through
  }
  return null;
}

function formatTimestamp(seconds: number): string {
  if (!seconds)
    return '—';
  try {
    return new Date(seconds * 1000).toLocaleString();
  }
  catch {
    return String(seconds);
  }
}

export function SettingsPage() {
  const toaster = useToaster();
  const { t } = useTranslation();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const ss = useSsStatusQuery();
  const wg = useWgStatusQuery();
  const settings = useSettingsQuery();
  const update = useUpdateSettingsMutation();
  const reload = useReloadMutation();

  const observed = useMemo(() => {
    return {
      hostname: settings.data?.domain ?? ss.data?.public_host ?? wg.data?.endpoint_host ?? '',
      wgSubnet: settings.data?.wg_subnet ?? wg.data?.subnet ?? '',
      ssPort: settings.data?.ss_listen_port ?? ss.data?.listen_port ?? 0,
      wgPort: settings.data?.wg_listen_port ?? wg.data?.listen_port ?? 0,
    };
  }, [settings.data, ss.data, wg.data]);

  const [hostname, setHostname] = useState(observed.hostname);
  const [wgSubnet, setWgSubnet] = useState(observed.wgSubnet);
  const [ssPort, setSsPort] = useState<string>(String(observed.ssPort || ''));
  const [wgPort, setWgPort] = useState<string>(String(observed.wgPort || ''));
  const [adminPassword, setAdminPassword] = useState('');
  const [adminPasswordConfirm, setAdminPasswordConfirm] = useState('');

  useEffect(() => {
    /* eslint-disable react/set-state-in-effect */
    setHostname(observed.hostname);
    setWgSubnet(observed.wgSubnet);
    setSsPort(observed.ssPort ? String(observed.ssPort) : '');
    setWgPort(observed.wgPort ? String(observed.wgPort) : '');
    /* eslint-enable react/set-state-in-effect */
  }, [observed.hostname, observed.wgSubnet, observed.ssPort, observed.wgPort]);

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (adminPassword && adminPassword !== adminPasswordConfirm) {
      toaster.error(
        t('settings.toasts.passwordMismatchTitle'),
        t('settings.toasts.passwordMismatchBody'),
      );
      return;
    }

    const patch: ServerSettingsPatch = {};
    const currentDomain = settings.data?.domain ?? null;
    const nextDomain = hostname.trim() === '' ? null : hostname.trim();
    if (nextDomain !== currentDomain)
      patch.domain = nextDomain;

    const currentSubnet = settings.data?.wg_subnet ?? null;
    const nextSubnet = wgSubnet.trim() === '' ? null : wgSubnet.trim();
    if (nextSubnet !== currentSubnet)
      patch.wg_subnet = nextSubnet;

    const parsedSsPort = Number.parseInt(ssPort, 10);
    if (
      Number.isFinite(parsedSsPort)
      && parsedSsPort > 0
      && parsedSsPort !== settings.data?.ss_listen_port
    ) {
      patch.ss_listen_port = parsedSsPort;
    }
    const parsedWgPort = Number.parseInt(wgPort, 10);
    if (
      Number.isFinite(parsedWgPort)
      && parsedWgPort > 0
      && parsedWgPort !== settings.data?.wg_listen_port
    ) {
      patch.wg_listen_port = parsedWgPort;
    }

    const credentialsChanged = Boolean(adminPassword);
    if (adminPassword)
      patch.new_password = adminPassword;

    if (Object.keys(patch).length === 0) {
      toaster.success(t('settings.toasts.saved'));
      return;
    }

    update.mutate(patch, {
      onSuccess: () => {
        toaster.success(t('settings.toasts.saved'));
        setAdminPassword('');
        setAdminPasswordConfirm('');
        if (credentialsChanged) {
          // Drop any refetches the hook-level onSuccess kicked off:
          // once we clear the token they would 401 and surface as
          // spurious error toasts. Also prevents the next admin from
          // seeing the previous session's cached data.
          qc.cancelQueries();
          qc.clear();
          authStore.clear();
          toaster.success(
            t('settings.toasts.credentialsRotatedTitle'),
            t('settings.toasts.credentialsRotatedBody'),
          );
          void navigate({ to: '/login' });
        }
      },
      onError: (err) => {
        const conflicts = parseSubnetConflict(err);
        if (conflicts) {
          toaster.error(
            t('settings.toasts.subnetConflictTitle'),
            t('settings.toasts.subnetConflictBody', {
              count: conflicts.length,
              ids: conflicts.join(', '),
            }),
          );
          return;
        }
        toaster.error(t('settings.toasts.updateFailed'), err.message);
      },
    });
  };

  const onReload = () => {
    reload.mutate(undefined, {
      onSuccess: () => toaster.success(t('settings.toasts.reloaded')),
      onError: (err) => toaster.error(t('settings.toasts.reloadFailed'), err.message),
    });
  };

  return (
    <div className="grid gap-4">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">{t('settings.heading')}</h1>
          <p className="text-sm text-muted-foreground">{t('settings.subtitle')}</p>
        </div>
        <div className="flex flex-col items-end gap-1">
          <Button
            type="button"
            variant="outline"
            onClick={onReload}
            disabled={reload.isPending}
            aria-label={t('settings.reload')}
          >
            <RefreshCw className={`mr-2 h-4 w-4 ${reload.isPending ? 'animate-spin' : ''}`} />
            {reload.isPending ? t('settings.reloading') : t('settings.reload')}
          </Button>
          <p className="max-w-[22rem] text-right text-xs text-muted-foreground">
            {t('settings.reloadHelp')}
          </p>
        </div>
      </div>

      <form onSubmit={onSubmit} className="grid gap-4">
        <Card>
          <CardHeader>
            <CardTitle className="text-base">{t('settings.network.title')}</CardTitle>
            <CardDescription>{t('settings.network.subtitle')}</CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <div className="grid gap-2">
              <Label htmlFor="hostname">{t('settings.network.domainLabel')}</Label>
              <Input
                id="hostname"
                value={hostname}
                onChange={(e) => setHostname(e.target.value)}
                placeholder={t('settings.network.domainPlaceholder')}
              />
              <p className="text-xs text-muted-foreground">{t('settings.network.domainHelp')}</p>
            </div>
            <div className="grid gap-2">
              <Label htmlFor="wg-subnet">{t('settings.network.subnetLabel')}</Label>
              <Input
                id="wg-subnet"
                value={wgSubnet}
                onChange={(e) => setWgSubnet(e.target.value)}
                placeholder={t('settings.network.subnetPlaceholder')}
              />
              <p className="text-xs text-muted-foreground">{t('settings.network.subnetHelp')}</p>
            </div>
            <Separator />
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="grid gap-2">
                <Label htmlFor="ss-port">{t('settings.network.ssPortLabel')}</Label>
                <Input
                  id="ss-port"
                  type="number"
                  min={1}
                  max={65535}
                  value={ssPort}
                  onChange={(e) => setSsPort(e.target.value)}
                />
                <p className="text-xs text-muted-foreground">{t('settings.network.ssPortHelp')}</p>
              </div>
              <div className="grid gap-2">
                <Label htmlFor="wg-port">{t('settings.network.wgPortLabel')}</Label>
                <Input
                  id="wg-port"
                  type="number"
                  min={1}
                  max={65535}
                  value={wgPort}
                  onChange={(e) => setWgPort(e.target.value)}
                />
                <p className="text-xs text-muted-foreground">{t('settings.network.wgPortHelp')}</p>
              </div>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-base">{t('settings.credentials.title')}</CardTitle>
            <CardDescription>
              <Trans
                i18nKey="settings.credentials.description"
                components={{ 1: <code className="font-mono" /> }}
              />
            </CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <div className="grid gap-2">
              <Label htmlFor="new-password">{t('settings.credentials.newPassword')}</Label>
              <Input
                id="new-password"
                type="password"
                autoComplete="new-password"
                value={adminPassword}
                onChange={(e) => setAdminPassword(e.target.value)}
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="new-password-confirm">
                {t('settings.credentials.confirmPassword')}
              </Label>
              <Input
                id="new-password-confirm"
                type="password"
                autoComplete="new-password"
                value={adminPasswordConfirm}
                onChange={(e) => setAdminPasswordConfirm(e.target.value)}
              />
            </div>
          </CardContent>
        </Card>

        {settings.data
          ? (
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">{t('settings.status.heading')}</CardTitle>
                  <CardDescription>{t('settings.status.description')}</CardDescription>
                </CardHeader>
                <CardContent className="grid gap-3 text-sm sm:grid-cols-2">
                  <div className="grid gap-0.5">
                    <dt className="text-xs text-muted-foreground">
                      {t('settings.status.tokenGeneration')}
                    </dt>
                    <dd className="font-mono">{settings.data.token_generation}</dd>
                    <p className="text-[11px] leading-snug text-muted-foreground">
                      {t('settings.status.tokenGenerationHelp')}
                    </p>
                  </div>
                  <div className="grid gap-0.5">
                    <dt className="text-xs text-muted-foreground">
                      {t('settings.status.updatedAt')}
                    </dt>
                    <dd className="font-mono">{formatTimestamp(settings.data.updated_at)}</dd>
                    <p className="text-[11px] leading-snug text-muted-foreground">
                      {t('settings.status.updatedAtHelp')}
                    </p>
                  </div>
                </CardContent>
              </Card>
            )
          : null}

        <div className="flex items-center justify-end gap-2">
          <Button type="submit" disabled={update.isPending}>
            <Save className="mr-2 h-4 w-4" />
            {update.isPending ? t('settings.savingProgress') : t('settings.save')}
          </Button>
        </div>
      </form>
    </div>
  );
}

export default SettingsPage;
