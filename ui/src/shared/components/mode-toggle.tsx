import type { Theme } from '@/shared/components/theme-provider';
import { Check, Monitor, Moon, Sun } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useTheme } from '@/shared/components/theme-provider';
import { Button } from '@/shared/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/shared/components/ui/dropdown-menu';
import { cn } from '@/shared/lib/utils';

const OPTIONS: Array<{ value: Theme; labelKey: string; icon: typeof Sun }> = [
  { value: 'light', labelKey: 'common.theme.light', icon: Sun },
  { value: 'dark', labelKey: 'common.theme.dark', icon: Moon },
  { value: 'system', labelKey: 'common.theme.system', icon: Monitor },
];

export function ModeToggle() {
  const { theme, setTheme } = useTheme();
  const { t } = useTranslation();

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          aria-label={t('common.theme.label')}
          className="h-8 w-8 px-0"
        >
          <Sun className="h-4 w-4 scale-100 rotate-0 transition-all dark:scale-0 dark:-rotate-90" />
          <Moon className="absolute h-4 w-4 scale-0 rotate-90 transition-all dark:scale-100 dark:rotate-0" />
          <span className="sr-only">{t('common.theme.label')}</span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent>
        {OPTIONS.map(({ value, labelKey, icon: Icon }) => {
          const active = theme === value;
          return (
            <DropdownMenuItem key={value} onClick={() => setTheme(value)}>
              <Icon className="h-3.5 w-3.5" aria-hidden="true" />
              <span className="flex-1">{t(labelKey)}</span>
              <Check
                className={cn('h-3.5 w-3.5', active ? 'opacity-100' : 'opacity-0')}
                aria-hidden="true"
              />
            </DropdownMenuItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export default ModeToggle;
