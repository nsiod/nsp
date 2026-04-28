// shadcn-style wrapper around @base-ui-components/react/menu. Public
// API mirrors what call sites already expect (Trigger / Content /
// Item / Separator / Label) while delegating positioning + portal
// management to base-ui.

import type { ComponentProps, ReactElement, ReactNode } from 'react';
import { Menu as MenuPrimitive } from '@base-ui-components/react/menu';
import { isValidElement } from 'react';
import { cn } from '@/shared/lib/utils';

export function DropdownMenu(props: ComponentProps<typeof MenuPrimitive.Root>) {
  return <MenuPrimitive.Root {...props} />;
}

interface DropdownMenuTriggerProps
  extends ComponentProps<typeof MenuPrimitive.Trigger> {
  asChild?: boolean;
  children?: ReactNode;
}

export function DropdownMenuTrigger({
  asChild,
  children,
  ...props
}: DropdownMenuTriggerProps) {
  if (asChild && isValidElement(children)) {
    return (
      <MenuPrimitive.Trigger
        data-slot="dropdown-menu-trigger"
        {...props}
        render={children as ReactElement<Record<string, unknown>>}
      />
    );
  }
  return (
    <MenuPrimitive.Trigger data-slot="dropdown-menu-trigger" {...props}>
      {children}
    </MenuPrimitive.Trigger>
  );
}

export function DropdownMenuPortal(
  props: ComponentProps<typeof MenuPrimitive.Portal>,
) {
  return <MenuPrimitive.Portal {...props} />;
}

interface DropdownMenuContentProps
  extends ComponentProps<typeof MenuPrimitive.Popup> {
  align?: 'start' | 'center' | 'end';
  sideOffset?: number;
}

export function DropdownMenuContent({
  className,
  align = 'end',
  sideOffset = 4,
  ...props
}: DropdownMenuContentProps) {
  return (
    <MenuPrimitive.Portal>
      <MenuPrimitive.Positioner align={align} sideOffset={sideOffset}>
        <MenuPrimitive.Popup
          data-slot="dropdown-menu-content"
          className={cn(
            'z-50 min-w-[8rem] overflow-hidden rounded-md border border-border bg-card p-1 text-card-foreground shadow-md outline-none',
            'data-[open]:animate-in data-[closed]:animate-out data-[closed]:fade-out-0 data-[open]:fade-in-0',
            className,
          )}
          {...props}
        />
      </MenuPrimitive.Positioner>
    </MenuPrimitive.Portal>
  );
}

interface DropdownMenuItemProps
  extends ComponentProps<typeof MenuPrimitive.Item> {
  inset?: boolean;
}

export function DropdownMenuItem({
  className,
  inset = false,
  ...props
}: DropdownMenuItemProps) {
  return (
    <MenuPrimitive.Item
      data-slot="dropdown-menu-item"
      data-inset={inset || undefined}
      className={cn(
        'relative flex cursor-pointer select-none items-center gap-2 rounded-sm px-2 py-1.5 text-xs outline-none',
        'data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground',
        'data-[disabled]:pointer-events-none data-[disabled]:opacity-50',
        inset && 'pl-8',
        className,
      )}
      {...props}
    />
  );
}

export function DropdownMenuSeparator({
  className,
  ...props
}: ComponentProps<typeof MenuPrimitive.Separator>) {
  return (
    <MenuPrimitive.Separator
      data-slot="dropdown-menu-separator"
      className={cn('-mx-1 my-1 h-px bg-border', className)}
      {...props}
    />
  );
}

export function DropdownMenuLabel({
  className,
  inset = false,
  ...props
}: ComponentProps<typeof MenuPrimitive.GroupLabel> & { inset?: boolean }) {
  return (
    <MenuPrimitive.GroupLabel
      data-slot="dropdown-menu-label"
      data-inset={inset || undefined}
      className={cn(
        'px-2 py-1.5 text-xs font-medium text-muted-foreground',
        inset && 'pl-8',
        className,
      )}
      {...props}
    />
  );
}
