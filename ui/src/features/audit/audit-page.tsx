// Read-only audit log viewer for the protected `/api/audit` endpoint.

import { FileClock } from 'lucide-react';
import { useState } from 'react';
import { Trans, useTranslation } from 'react-i18next';
import { useAuditQuery } from '@/features/audit/api';
import { EmptyState } from '@/shared/components/empty-state';
import { Badge } from '@/shared/components/ui/badge';
import { Button } from '@/shared/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/components/ui/card';
import { Input } from '@/shared/components/ui/input';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/shared/components/ui/table';
import { formatTimestamp } from '@/shared/lib/utils';

export function AuditPage() {
  const audit = useAuditQuery();
  const { t } = useTranslation();
  const [filter, setFilter] = useState('');

  const unavailable = audit.error?.isUnavailable() ?? false;

  const rows
    = audit.data?.filter((e) => {
      if (!filter)
        return true;
      const q = filter.toLowerCase();
      return (
        e.actor.toLowerCase().includes(q)
        || e.action.toLowerCase().includes(q)
        || (e.target ?? '').toLowerCase().includes(q)
        || (e.detail ?? '').toLowerCase().includes(q)
      );
    }) ?? [];

  return (
    <div className="grid gap-4">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">{t('audit.heading')}</h1>
        <p className="text-sm text-muted-foreground">
          <Trans i18nKey="audit.description" components={{ 1: <code className="font-mono" /> }} />
        </p>
      </div>

      {unavailable
        ? (
            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2 text-base">
                  <FileClock className="h-4 w-4 text-muted-foreground" />
                  {t('audit.unavailableTitle')}
                </CardTitle>
                <CardDescription>
                  <Trans
                    i18nKey="audit.unavailableBody"
                    components={{
                      1: <code className="font-mono" />,
                      3: <code className="font-mono" />,
                    }}
                  />
                </CardDescription>
              </CardHeader>
              <CardContent>
                <Button variant="outline" size="sm" onClick={() => audit.refetch()}>
                  {t('common.retry')}
                </Button>
              </CardContent>
            </Card>
          )
        : (
            <>
              <div className="flex items-center gap-2">
                <Input
                  value={filter}
                  onChange={(e) => setFilter(e.target.value)}
                  placeholder={t('audit.filterPlaceholder')}
                  className="max-w-sm"
                  aria-label={t('audit.filterAria')}
                />
                <Badge variant="muted">{t('audit.entryCount', { count: rows.length })}</Badge>
              </div>
              <div className="rounded-md border border-border bg-card">
                {audit.isLoading
                  ? (
                      <EmptyState
                        icon={<FileClock className="h-8 w-8" />}
                        title={t('common.loading')}
                      />
                    )
                  : rows.length === 0
                    ? (
                        <EmptyState
                          icon={<FileClock className="h-8 w-8" />}
                          title={filter ? t('audit.emptyFiltered') : t('audit.empty')}
                          description={!filter ? t('audit.emptyDescription') : undefined}
                        />
                      )
                    : (
                        <Table>
                          <TableHeader>
                            <TableRow>
                              <TableHead>{t('audit.table.timestamp')}</TableHead>
                              <TableHead>{t('audit.table.actor')}</TableHead>
                              <TableHead>{t('audit.table.action')}</TableHead>
                              <TableHead>{t('audit.table.target')}</TableHead>
                              <TableHead>{t('audit.table.detail')}</TableHead>
                            </TableRow>
                          </TableHeader>
                          <TableBody>
                            {rows.map((e) => (
                              <TableRow key={e.id}>
                                <TableCell className="font-mono text-xs text-muted-foreground">
                                  {formatTimestamp(e.ts)}
                                </TableCell>
                                <TableCell>{e.actor}</TableCell>
                                <TableCell className="font-mono text-xs">{e.action}</TableCell>
                                <TableCell className="font-mono text-xs text-muted-foreground">
                                  {e.target ?? '—'}
                                </TableCell>
                                <TableCell className="max-w-[24rem] truncate text-xs text-muted-foreground">
                                  {e.detail ?? '—'}
                                </TableCell>
                              </TableRow>
                            ))}
                          </TableBody>
                        </Table>
                      )}
              </div>
            </>
          )}
    </div>
  );
}

export default AuditPage;
