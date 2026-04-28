// Native <select> styled to match Input. We deliberately stay on the
// platform control here: the iptables dialog needs a short, fixed list
// (5 tables, ≤5 chains per table) where the OS picker beats a custom
// popup on accessibility, mobile, and code size.

import type { ComponentProps } from 'react';
import { cn } from '@/shared/lib/utils';

export function Select({ className, ...props }: ComponentProps<'select'>) {
  return (
    <select
      data-slot="select"
      className={cn(
        'flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm',
        'focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
        'disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...props}
    />
  );
}
