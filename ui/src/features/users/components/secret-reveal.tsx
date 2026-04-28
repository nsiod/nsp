// One-time secret display dialog. Used after create/rotate to show the key
// material a single time with copy + download + QR. The dialog is the *only*
// place these values are visible — nothing here is persisted client-side.

import { ChevronDown, ChevronRight, Copy, Download, QrCode } from 'lucide-react';
import QRCode from 'qrcode';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/shared/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/shared/components/ui/dialog';
import { Separator } from '@/shared/components/ui/separator';
import { useToaster } from '@/shared/components/ui/toast';
import { fetchBinary } from '@/shared/lib/http';
import { cn } from '@/shared/lib/utils';

export interface SecretBlock {
  label: string;
  /** Body text; rendered in a monospaced code block. */
  value: string;
  /** Suggested filename when the user clicks Download. */
  filename: string;
  /** Mime hint for the download. Defaults to text/plain. */
  mime?: string;
  /**
   * Optional API path returning a PNG QR for this material. Used when
   * the server can regenerate the encoded payload (e.g. SS URLs). For
   * WG conf — where the server cannot reconstruct the private half —
   * use `qrData` instead.
   */
  qrPath?: string;
  /**
   * Optional string to render locally as a QR code. Mutually exclusive
   * with `qrPath`; when both are set `qrPath` wins.
   */
  qrData?: string;
  /**
   * When true the block is hidden behind an "advanced" toggle. Use for
   * material that most operators don't need (raw keys when the primary
   * URL / QR already covers them).
   */
  advanced?: boolean;
}

interface SecretRevealProps {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: string;
  blocks: SecretBlock[];
}

export function SecretReveal({ open, onClose, title, description, blocks }: SecretRevealProps) {
  const { t } = useTranslation();
  const primary = blocks.filter((b) => !b.advanced);
  const advanced = blocks.filter((b) => b.advanced);
  const [showAdvanced, setShowAdvanced] = useState(false);

  // Reset the advanced toggle every time the dialog re-opens so previous
  // reveal state does not leak into the next credential display.
  useEffect(() => {
    if (open) {
      // eslint-disable-next-line react/set-state-in-effect
      setShowAdvanced(false);
    }
  }, [open]);

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o)
          onClose();
      }}
    >
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            {description ?? t('common.secretReveal.defaultDescription')}
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-4">
          {primary.map((b) => (
            <SecretBlockView key={b.label} block={b} />
          ))}
          {advanced.length > 0
            ? (
                <div className="grid gap-3">
                  <button
                    type="button"
                    onClick={() => setShowAdvanced((v) => !v)}
                    className="inline-flex w-fit items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
                  >
                    {showAdvanced
                      ? (
                          <ChevronDown className="h-3.5 w-3.5" />
                        )
                      : (
                          <ChevronRight className="h-3.5 w-3.5" />
                        )}
                    {showAdvanced
                      ? t('common.secretReveal.hideAdvanced')
                      : t('common.secretReveal.showAdvanced')}
                  </button>
                  {showAdvanced
                    ? advanced.map((b) => <SecretBlockView key={b.label} block={b} />)
                    : null}
                </div>
              )
            : null}
        </div>

        <DialogFooter>
          <Button variant="secondary" onClick={onClose}>
            {t('common.secretReveal.done')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function SecretBlockView({ block }: { block: SecretBlock }) {
  const toaster = useToaster();
  const { t } = useTranslation();
  const [qrUrl, setQrUrl] = useState<string | null>(null);
  const [qrLoading, setQrLoading] = useState(false);

  useEffect(() => {
    return () => {
      if (qrUrl)
        URL.revokeObjectURL(qrUrl);
    };
  }, [qrUrl]);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(block.value);
      toaster.success(
        t('common.secretReveal.copied'),
        t('common.secretReveal.copiedBody', { label: block.label }),
      );
    }
    catch (err) {
      toaster.error(
        t('common.secretReveal.copyFailed'),
        err instanceof Error ? err.message : String(err),
      );
    }
  };

  const handleDownload = () => {
    const mime = block.mime ?? 'text/plain';
    const blob = new Blob([block.value], { type: mime });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = block.filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  };

  const handleQr = async () => {
    setQrLoading(true);
    try {
      if (block.qrPath) {
        const { objectUrl } = await fetchBinary(block.qrPath);
        setQrUrl((prev) => {
          if (prev)
            URL.revokeObjectURL(prev);
          return objectUrl;
        });
        return;
      }
      if (block.qrData) {
        // Render locally — no private material leaves the browser.
        const dataUrl = await QRCode.toDataURL(block.qrData, {
          errorCorrectionLevel: 'M',
          margin: 1,
          width: 320,
        });
        setQrUrl((prev) => {
          if (prev)
            URL.revokeObjectURL(prev);
          return dataUrl;
        });
      }
    }
    catch (err) {
      toaster.error(
        t('common.secretReveal.qrFailed'),
        err instanceof Error ? err.message : String(err),
      );
    }
    finally {
      setQrLoading(false);
    }
  };

  return (
    <div className="rounded-md border border-border bg-background/40">
      <div className="flex items-center justify-between gap-2 border-b border-border px-3 py-2">
        <span className="text-sm font-medium">{block.label}</span>
        <div className="flex items-center gap-1">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={handleCopy}
            aria-label={t('common.secretReveal.copyAria', { label: block.label })}
          >
            <Copy className="h-3.5 w-3.5" />
            <span className="ml-1.5">{t('common.secretReveal.copy')}</span>
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={handleDownload}
            aria-label={t('common.secretReveal.downloadAria', { label: block.label })}
          >
            <Download className="h-3.5 w-3.5" />
            <span className="ml-1.5">{t('common.secretReveal.download')}</span>
          </Button>
          {block.qrPath || block.qrData
            ? (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={handleQr}
                  disabled={qrLoading}
                  aria-label={t('common.secretReveal.showQrAria', { label: block.label })}
                >
                  <QrCode className="h-3.5 w-3.5" />
                  <span className="ml-1.5">
                    {qrUrl ? t('common.secretReveal.reloadQr') : t('common.secretReveal.showQr')}
                  </span>
                </Button>
              )
            : null}
        </div>
      </div>
      <div className={cn('px-3 py-2 text-xs', qrUrl ? 'grid gap-3 sm:grid-cols-[1fr,auto]' : '')}>
        <Separator className="hidden" />
        <pre className="overflow-x-auto whitespace-pre-wrap break-all font-mono text-xs leading-relaxed text-muted-foreground">
          {block.value}
        </pre>
        {qrUrl
          ? (
              <img
                src={qrUrl}
                alt={t('common.secretReveal.qrAlt', { label: block.label })}
                className="h-40 w-40 self-start rounded-md border border-border bg-[hsl(0_0%_100%)] p-1"
              />
            )
          : null}
      </div>
    </div>
  );
}
