// Dropdown-style language picker for the header. The active language is
// written to localStorage by i18next-browser-languagedetector via
// `changeLanguage`, so reloading the page picks up the same choice.

import type { SupportedLocale } from '@/app/i18n';
import { Check, ChevronDown, Languages } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { supportedLocales } from '@/app/i18n';
import { Button } from '@/shared/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/shared/components/ui/dropdown-menu';
import { cn } from '@/shared/lib/utils';

function labelFor(code: SupportedLocale, t: (key: string) => string): string {
  return code === 'zh-CN' ? t('common.languageChinese') : t('common.languageEnglish');
}

export function LanguageSwitcher() {
  const { i18n, t } = useTranslation();
  const active = (i18n.resolvedLanguage ?? i18n.language ?? 'en') as SupportedLocale;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          aria-label={t('common.language')}
          className="h-8 gap-1.5 px-2 text-xs"
        >
          <Languages className="h-3.5 w-3.5 text-muted-foreground" aria-hidden="true" />
          <span>{labelFor(active, t)}</span>
          <ChevronDown className="h-3 w-3 text-muted-foreground" aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent>
        {supportedLocales.map((code) => {
          const isActive = active === code;
          return (
            <DropdownMenuItem
              key={code}
              onClick={() => {
                if (!isActive)
                  void i18n.changeLanguage(code);
              }}
            >
              <Check
                className={cn('h-3.5 w-3.5', isActive ? 'opacity-100' : 'opacity-0')}
                aria-hidden="true"
              />
              <span>{labelFor(code, t)}</span>
            </DropdownMenuItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export default LanguageSwitcher;
