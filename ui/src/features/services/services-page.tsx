import type { ServiceDescriptor } from '@/features/services/descriptors';
import { Link } from '@tanstack/react-router';
import { ChevronRight, Play, Square } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { serviceDescriptors } from '@/features/services/descriptors';
import { Badge } from '@/shared/components/ui/badge';
import { Button, buttonVariants } from '@/shared/components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from '@/shared/components/ui/card';
import { useToaster } from '@/shared/components/ui/toast';

const METRIC_LABEL_KEYS: Record<string, string> = {
  'Users': 'services.metrics.users',
  'Reloads': 'services.metrics.reloads',
  'Last swap': 'services.metrics.lastSwap',
  'Peers': 'services.metrics.peers',
  'Listen port': 'services.metrics.listenPort',
  'Endpoint': 'services.metrics.endpoint',
  'SOCKS5 port': 'services.metrics.socks5Port',
  'HTTP port': 'services.metrics.httpPort',
};

export function ServicesPage() {
  const { t } = useTranslation();
  return (
    <div className="grid gap-4">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">{t('services.heading')}</h1>
        <p className="text-sm text-muted-foreground">{t('services.subtitle')}</p>
      </div>
      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
        {serviceDescriptors.map((svc) => (
          <ServiceCard key={svc.id} svc={svc} />
        ))}
      </div>
    </div>
  );
}

function ServiceCard({ svc }: { svc: ServiceDescriptor }) {
  const toaster = useToaster();
  const { t } = useTranslation();
  const status = svc.useStatusQuery();
  const start = svc.useStart();
  const stop = svc.useStop();

  const busy = start.isPending || stop.isPending;
  const data = status.data;
  const running = data?.running ?? false;
  const available = data?.available ?? true;
  const reason = data?.reason ?? null;
  const driverDown = status.error?.isUnavailable() ?? false;

  const descriptionKey = `services.descriptions.${svc.id}` as const;
  const description = t(descriptionKey);

  const onStart = () => {
    start.mutate(undefined, {
      onError: (err) => toaster.error(t('services.startFailed', { name: svc.name }), err.message),
    });
  };

  const onStop = () => {
    stop.mutate(undefined, {
      onError: (err) => toaster.error(t('services.stopFailed', { name: svc.name }), err.message),
    });
  };

  return (
    <Card className="transition-shadow hover:shadow-md">
      <CardHeader>
        <div className="flex items-start justify-between gap-3">
          <div>
            <CardTitle className="flex items-center gap-2">
              <Link
                to="/services/$id"
                params={{ id: svc.id }}
                className="hover:underline"
                aria-label={t('services.detailAria', { name: svc.name })}
              >
                {svc.name}
              </Link>
              {running
                ? (
                    <Badge variant="success">{t('services.running')}</Badge>
                  )
                : (
                    <Badge variant="muted">{t('services.stopped')}</Badge>
                  )}
            </CardTitle>
            <CardDescription>{description}</CardDescription>
          </div>
          <Link
            to="/services/$id"
            params={{ id: svc.id }}
            aria-label={t('services.detailAria', { name: svc.name })}
            className={buttonVariants({ variant: 'ghost', size: 'sm' })}
          >
            {t('services.detail')}
            <ChevronRight className="ml-1 h-3.5 w-3.5" />
          </Link>
        </div>
      </CardHeader>
      <CardContent className="grid gap-3 text-sm">
        {driverDown
          ? (
              <div className="rounded-md border border-amber-300 bg-amber-50 p-3 text-xs text-amber-900 dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-200">
                {t('services.driverDown')}
              </div>
            )
          : status.isLoading
            ? (
                <div className="text-muted-foreground">{t('services.loading')}</div>
              )
            : data
              ? (
                  <>
                    {data.subtitle
                      ? (
                          <div className="font-mono text-xs text-muted-foreground">{data.subtitle}</div>
                        )
                      : null}
                    {data.metrics.length > 0 && (
                      <dl className="grid grid-cols-3 gap-2 rounded-md border border-border p-3">
                        {data.metrics.map((m) => {
                          const key = METRIC_LABEL_KEYS[m.label];
                          const label = key ? t(key) : m.label;
                          return (
                            <div key={m.label} className="grid gap-0.5">
                              <dt className="text-[10px] uppercase tracking-wide text-muted-foreground">
                                {label}
                              </dt>
                              <dd className="font-mono text-xs">{m.value}</dd>
                            </div>
                          );
                        })}
                      </dl>
                    )}
                    {!running && !available && reason
                      ? (
                          <div className="text-xs text-muted-foreground">
                            {t('services.preconditions')}
                            {' '}
                            <span className="font-mono">{reason}</span>
                          </div>
                        )
                      : null}
                  </>
                )
              : null}
      </CardContent>
      <CardFooter className="flex items-center justify-end gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={onStart}
          disabled={driverDown || busy || running || !available}
          aria-label={t('services.startAria', { name: svc.name })}
        >
          <Play className="mr-2 h-4 w-4" />
          {t('services.start')}
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={onStop}
          disabled={driverDown || busy || !running}
          aria-label={t('services.stopAria', { name: svc.name })}
        >
          <Square className="mr-2 h-4 w-4" />
          {t('services.stop')}
        </Button>
      </CardFooter>
    </Card>
  );
}

export default ServicesPage;
