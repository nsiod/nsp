import type { ComponentProps, ReactNode } from 'react';
import { cn } from '@/shared/lib/utils';

interface EmptyStateProps extends ComponentProps<'div'> {
  icon?: ReactNode;
  title: string;
  description?: ReactNode;
  action?: ReactNode;
}

/**
 * Centered "nothing to see here" surface for empty tables / lists.
 * Slot in an icon (lucide preferred), a title, optional description,
 * and an optional CTA. Stays inside whatever container it sits in.
 */
export function EmptyState({
  icon,
  title,
  description,
  action,
  className,
  ...props
}: EmptyStateProps) {
  return (
    <div
      data-slot="empty-state"
      className={cn(
        'flex flex-col items-center justify-center gap-2 px-6 py-12 text-center',
        className,
      )}
      {...props}
    >
      {icon
        ? (
            <div className="text-muted-foreground/60" aria-hidden="true">
              {icon}
            </div>
          )
        : null}
      <p className="text-sm font-medium text-foreground">{title}</p>
      {description
        ? (
            <p className="max-w-md text-xs text-muted-foreground">{description}</p>
          )
        : null}
      {action
        ? (
            <div className="mt-2">{action}</div>
          )
        : null}
    </div>
  );
}
