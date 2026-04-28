import type { ClassValue } from 'clsx';
import { clsx } from 'clsx';
import { twMerge } from 'tailwind-merge';

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}

export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0)
    return '0 B';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  return `${(n / 1024 ** i).toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

export function formatRelativeSeconds(secs: number | null | undefined): string {
  if (secs == null)
    return 'never';
  if (secs < 60)
    return `${secs}s ago`;
  if (secs < 3600)
    return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400)
    return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)}d ago`;
}

export function formatTimestamp(unixSeconds: number): string {
  const d = new Date(unixSeconds * 1000);
  return `${d.toISOString().replace('T', ' ').slice(0, 19)} UTC`;
}
