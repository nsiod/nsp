import type { FormEvent } from 'react';
import type { ApiError } from '@/shared/lib/http';
import type {
  IptablesCreateRequest,
  IptablesRule,
  IptablesSource,
  IptablesSshGuardBody,
} from '@/shared/types';
import { AlertTriangle, Plus, RefreshCw, ShieldAlert, Trash2 } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  useCreateIptablesRuleMutation,
  useDeleteIptablesRuleMutation,
  useIptablesRulesQuery,
  useReconcileIptablesRulesMutation,
} from '@/features/iptables/api';
import { EmptyState } from '@/shared/components/empty-state';
import { Badge } from '@/shared/components/ui/badge';
import { Button } from '@/shared/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/shared/components/ui/dialog';
import { Input } from '@/shared/components/ui/input';
import { Label } from '@/shared/components/ui/label';
import { Select } from '@/shared/components/ui/select';
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

// User-customisable iptables tables and their built-in chains. We
// intentionally drop `security` (rare, rule-of-thumb advanced) and
// `mangle`/`raw` PREROUTING/POSTROUTING are kept for completeness so
// the SS/WG forward path can be tweaked.
const IPTABLES_TABLES = ['filter', 'nat', 'mangle', 'raw'] as const;
type IptablesTable = (typeof IPTABLES_TABLES)[number];

const CHAINS_BY_TABLE: Record<IptablesTable, readonly string[]> = {
  filter: ['INPUT', 'FORWARD', 'OUTPUT'],
  nat: ['PREROUTING', 'INPUT', 'OUTPUT', 'POSTROUTING'],
  mangle: ['PREROUTING', 'INPUT', 'FORWARD', 'OUTPUT', 'POSTROUTING'],
  raw: ['PREROUTING', 'OUTPUT'],
};

const SOURCE_FILTER_OPTIONS: Array<{ key: 'all' | IptablesSource; label: string }> = [
  { key: 'all', label: 'iptables.filter.all' },
  { key: 'user', label: 'iptables.filter.user' },
  { key: 'wg-driver', label: 'iptables.filter.wgDriver' },
];

function parseSshGuard(err: ApiError): IptablesSshGuardBody | null {
  if (err.status !== 409 || err.code !== 'ssh-guard')
    return null;
  try {
    const parsed = JSON.parse(err.body) as Partial<IptablesSshGuardBody>;
    if (parsed?.code === 'ssh-guard' && typeof parsed.warn === 'string') {
      return { code: 'ssh-guard', warn: parsed.warn };
    }
  }
  catch {
    // fall through
  }
  return null;
}

export function IptablesPage() {
  const { t } = useTranslation();
  const toaster = useToaster();
  const [sourceFilter, setSourceFilter] = useState<'all' | IptablesSource>('all');
  const [createOpen, setCreateOpen] = useState(false);
  const [pendingForce, setPendingForce] = useState<{
    draft: IptablesCreateRequest;
    warn: string;
  } | null>(null);

  const listSource = sourceFilter === 'all' ? undefined : sourceFilter;
  const rules = useIptablesRulesQuery(listSource);
  const driverDown = rules.error?.isUnavailable() ?? false;

  const createRule = useCreateIptablesRuleMutation();
  const deleteRule = useDeleteIptablesRuleMutation();
  const reconcile = useReconcileIptablesRulesMutation();

  const ordered = useMemo(() => rules.data ?? [], [rules.data]);

  const handleCreate = (draft: IptablesCreateRequest, onDone?: () => void) => {
    createRule.mutate(draft, {
      onSuccess: () => {
        toaster.success(t('iptables.toasts.created'));
        setPendingForce(null);
        onDone?.();
      },
      onError: (err) => {
        const guard = parseSshGuard(err);
        if (guard) {
          setPendingForce({ draft, warn: guard.warn });
          return;
        }
        toaster.error(t('iptables.toasts.createFailed'), err.message);
      },
    });
  };

  const handleDelete = (rule: IptablesRule) => {
    if (!window.confirm(t('iptables.confirmDelete', { table: rule.table, chain: rule.chain }))) {
      return;
    }
    deleteRule.mutate(rule.id, {
      onSuccess: () => toaster.success(t('iptables.toasts.deleted')),
      onError: (err) => toaster.error(t('iptables.toasts.deleteFailed'), err.message),
    });
  };

  const handleReconcile = () => {
    reconcile.mutate(undefined, {
      onSuccess: (report) => {
        toaster.success(
          t('iptables.toasts.reconciled'),
          t('iptables.toasts.reconciledBody', {
            reinserted: report.reinserted,
            pruned: report.pruned,
            kept: report.kept,
          }),
        );
      },
      onError: (err) => toaster.error(t('iptables.toasts.reconcileFailed'), err.message),
    });
  };

  return (
    <div className="grid gap-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">{t('iptables.heading')}</h1>
          <p className="text-sm text-muted-foreground">{t('iptables.subtitle')}</p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={handleReconcile}
            disabled={driverDown || reconcile.isPending}
            aria-label={t('iptables.reconcileAria')}
          >
            <RefreshCw className="mr-2 h-4 w-4" />
            {reconcile.isPending ? t('iptables.reconciling') : t('iptables.reconcile')}
          </Button>
          <Button
            onClick={() => setCreateOpen(true)}
            disabled={driverDown}
            aria-label={t('iptables.newRuleAria')}
          >
            <Plus className="mr-2 h-4 w-4" />
            {t('iptables.newRule')}
          </Button>
        </div>
      </div>

      <div className="flex items-center gap-1">
        {SOURCE_FILTER_OPTIONS.map((opt) => (
          <Button
            key={opt.key}
            variant={sourceFilter === opt.key ? 'secondary' : 'ghost'}
            size="sm"
            onClick={() => setSourceFilter(opt.key)}
          >
            {t(opt.label)}
          </Button>
        ))}
      </div>

      {driverDown
        ? (
            <div className="rounded-md border border-amber-300 bg-amber-50 p-3 text-xs text-amber-900 dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-200">
              {t('iptables.driverDown')}
            </div>
          )
        : null}

      <Card>
        <CardHeader>
          <CardTitle className="text-base">{t('iptables.table.heading')}</CardTitle>
          <CardDescription>{t('iptables.table.description')}</CardDescription>
        </CardHeader>
        <CardContent className="p-0">
          {rules.isLoading
            ? (
                <EmptyState
                  icon={<ShieldAlert className="h-8 w-8" />}
                  title={t('common.loading')}
                />
              )
            : ordered.length === 0
              ? (
                  <EmptyState
                    icon={<ShieldAlert className="h-8 w-8" />}
                    title={t('iptables.empty')}
                    description={t('iptables.emptyDescription')}
                  />
                )
              : (
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>{t('iptables.table.source')}</TableHead>
                        <TableHead>{t('iptables.table.chain')}</TableHead>
                        <TableHead>{t('iptables.table.spec')}</TableHead>
                        <TableHead>{t('iptables.table.priority')}</TableHead>
                        <TableHead>{t('iptables.table.comment')}</TableHead>
                        <TableHead>{t('iptables.table.created')}</TableHead>
                        <TableHead className="text-right">{t('iptables.table.actions')}</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {ordered.map((rule) => (
                        <TableRow key={rule.id}>
                          <TableCell>
                            <SourceBadge source={rule.source} />
                          </TableCell>
                          <TableCell className="font-mono text-xs">
                            <div>{rule.table}</div>
                            <div className="text-muted-foreground">{rule.chain}</div>
                          </TableCell>
                          <TableCell className="font-mono text-xs">{rule.spec}</TableCell>
                          <TableCell className="font-mono text-xs">{rule.priority}</TableCell>
                          <TableCell className="text-xs text-muted-foreground">
                            {rule.comment ?? '—'}
                          </TableCell>
                          <TableCell className="text-xs text-muted-foreground">
                            {formatTimestamp(rule.created_at)}
                          </TableCell>
                          <TableCell className="text-right">
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => handleDelete(rule)}
                              disabled={rule.source !== 'user' || deleteRule.isPending}
                              aria-label={t('iptables.deleteAria', { id: rule.id })}
                            >
                              <Trash2 className="h-3.5 w-3.5 text-destructive" />
                            </Button>
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                )}
        </CardContent>
      </Card>

      <NewRuleDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        submitting={createRule.isPending}
        onSubmit={(draft) =>
          handleCreate(draft, () => {
            setCreateOpen(false);
          })}
      />

      {pendingForce
        ? (
            <SshGuardDialog
              warn={pendingForce.warn}
              submitting={createRule.isPending}
              onCancel={() => setPendingForce(null)}
              onConfirm={() => handleCreate({ ...pendingForce.draft, force: true })}
            />
          )
        : null}
    </div>
  );
}

function SourceBadge({ source }: { source: IptablesSource }) {
  const { t } = useTranslation();
  if (source === 'wg-driver') {
    return <Badge variant="secondary">{t('iptables.sources.wgDriver')}</Badge>;
  }
  return <Badge variant="success">{t('iptables.sources.user')}</Badge>;
}

interface NewRuleDialogProps {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  submitting: boolean;
  onSubmit: (draft: IptablesCreateRequest) => void;
}

function NewRuleDialog({ open, onOpenChange, submitting, onSubmit }: NewRuleDialogProps) {
  const { t } = useTranslation();
  const [table, setTable] = useState<IptablesTable>('filter');
  const [chain, setChain] = useState<string>('FORWARD');
  const [spec, setSpec] = useState('');
  const [comment, setComment] = useState('');
  const [priority, setPriority] = useState('0');

  const chains = CHAINS_BY_TABLE[table];

  const reset = () => {
    setTable('filter');
    setChain('FORWARD');
    setSpec('');
    setComment('');
    setPriority('0');
  };

  const handleTableChange = (next: IptablesTable) => {
    setTable(next);
    // Snap chain to the new table's first valid option if the previous
    // selection isn't legal under it (e.g. FORWARD doesn't exist in nat).
    if (!CHAINS_BY_TABLE[next].includes(chain))
      setChain(CHAINS_BY_TABLE[next][0]!);
  };

  const handleSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const trimmedSpec = spec.trim();
    if (!trimmedSpec)
      return;
    const parsedPriority = Number(priority);
    onSubmit({
      table,
      chain,
      spec: trimmedSpec,
      comment: comment.trim() ? comment.trim() : null,
      priority: Number.isFinite(parsedPriority) ? parsedPriority : 0,
    });
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(v) => {
        if (!v)
          reset();
        onOpenChange(v);
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t('iptables.dialog.title')}</DialogTitle>
          <DialogDescription>{t('iptables.dialog.description')}</DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="grid gap-3">
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-2">
              <Label htmlFor="ipt-table">{t('iptables.dialog.tableLabel')}</Label>
              <Select
                id="ipt-table"
                value={table}
                onChange={(e) => handleTableChange(e.target.value as IptablesTable)}
              >
                {IPTABLES_TABLES.map((name) => (
                  <option key={name} value={name}>{name}</option>
                ))}
              </Select>
            </div>
            <div className="grid gap-2">
              <Label htmlFor="ipt-chain">{t('iptables.dialog.chainLabel')}</Label>
              <Select
                id="ipt-chain"
                value={chain}
                onChange={(e) => setChain(e.target.value)}
              >
                {chains.map((name) => (
                  <option key={name} value={name}>{name}</option>
                ))}
              </Select>
            </div>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="ipt-spec">{t('iptables.dialog.specLabel')}</Label>
            <Input
              id="ipt-spec"
              value={spec}
              onChange={(e) => setSpec(e.target.value)}
              placeholder={t('iptables.dialog.specPlaceholder')}
              required
              className="font-mono"
            />
            <p className="text-xs text-muted-foreground">{t('iptables.dialog.specHelp')}</p>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-2">
              <Label htmlFor="ipt-priority">{t('iptables.dialog.priorityLabel')}</Label>
              <Input
                id="ipt-priority"
                type="number"
                value={priority}
                onChange={(e) => setPriority(e.target.value)}
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="ipt-comment">{t('iptables.dialog.commentLabel')}</Label>
              <Input
                id="ipt-comment"
                value={comment}
                onChange={(e) => setComment(e.target.value)}
                placeholder={t('iptables.dialog.commentPlaceholder')}
              />
            </div>
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              disabled={submitting}
              onClick={() => onOpenChange(false)}
            >
              {t('common.cancel')}
            </Button>
            <Button type="submit" disabled={submitting}>
              {submitting ? t('iptables.dialog.submitting') : t('iptables.dialog.submit')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

interface SshGuardDialogProps {
  warn: string;
  submitting: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}

function SshGuardDialog({ warn, submitting, onCancel, onConfirm }: SshGuardDialogProps) {
  const { t } = useTranslation();
  return (
    <Dialog open onOpenChange={(v) => !v && onCancel()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <AlertTriangle className="h-4 w-4 text-amber-600 dark:text-amber-400" />
            {t('iptables.sshGuard.title')}
          </DialogTitle>
          <DialogDescription>{t('iptables.sshGuard.description')}</DialogDescription>
        </DialogHeader>
        <div className="rounded-md border border-amber-300 bg-amber-50 p-3 text-xs text-amber-900 dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-200">
          {warn}
        </div>
        <DialogFooter>
          <Button type="button" variant="ghost" disabled={submitting} onClick={onCancel}>
            {t('common.cancel')}
          </Button>
          <Button type="button" variant="destructive" disabled={submitting} onClick={onConfirm}>
            {submitting ? t('iptables.sshGuard.confirming') : t('iptables.sshGuard.confirm')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export default IptablesPage;
