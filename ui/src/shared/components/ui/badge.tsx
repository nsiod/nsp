import type { VariantProps } from 'class-variance-authority';
import type { ComponentProps } from 'react';
import { cva } from 'class-variance-authority';
import { cn } from '@/shared/lib/utils';

export const badgeVariants = cva(
  'inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium transition-colors',
  {
    variants: {
      variant: {
        default: 'border-transparent bg-primary text-primary-foreground',
        secondary: 'border-transparent bg-secondary text-secondary-foreground',
        destructive: 'border-transparent bg-destructive text-destructive-foreground',
        outline: 'text-foreground',
        muted: 'border-transparent bg-muted text-muted-foreground',
        success:
          'border-transparent bg-emerald-100 text-emerald-800 dark:bg-emerald-600/20 dark:text-emerald-300',
      },
    },
    defaultVariants: { variant: 'default' },
  },
);

type BadgeProps = ComponentProps<'span'> & VariantProps<typeof badgeVariants>;

export function Badge({ className, variant, ...props }: BadgeProps) {
  return (
    <span
      data-slot="badge"
      className={cn(badgeVariants({ variant }), className)}
      {...props}
    />
  );
}

export type { BadgeProps };
