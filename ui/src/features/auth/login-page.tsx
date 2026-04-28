import type { FormEvent } from 'react';
import { useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useLoginMutation } from '@/features/auth/api';
import { Logo } from '@/shared/components/logo';
import { Button } from '@/shared/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/components/ui/card';
import { Input } from '@/shared/components/ui/input';
import { Label } from '@/shared/components/ui/label';
import { useToaster } from '@/shared/components/ui/toast';
import { authStore } from '@/shared/stores/auth';

export function LoginPage() {
  const navigate = useNavigate();
  const toaster = useToaster();
  const { t } = useTranslation();
  const [password, setPassword] = useState('');

  const login = useLoginMutation({
    onSuccess: ({ token, expires_at }) => {
      authStore.set(token, expires_at);
      void navigate({ to: '/users', replace: true });
    },
    onError: (err) => {
      toaster.error(t('login.failedTitle'), err.message);
    },
  });

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!password)
      return;
    login.mutate({ password });
  };

  return (
    <div className="flex min-h-full items-center justify-center px-4 py-10">
      <Card className="w-full max-w-sm">
        <CardHeader className="space-y-3 text-center">
          <Logo width={48} height={48} className="mx-auto" />
          <CardTitle className="text-lg">{t('login.title')}</CardTitle>
          <CardDescription>{t('login.description')}</CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={onSubmit} className="grid gap-4">
            <div className="grid gap-2">
              <Label htmlFor="password">{t('login.passwordLabel')}</Label>
              <Input
                id="password"
                type="password"
                value={password}
                autoComplete="current-password"
                onChange={(e) => setPassword(e.target.value)}
                required
                autoFocus
              />
            </div>
            <Button type="submit" className="w-full" disabled={login.isPending}>
              {login.isPending ? t('login.submitting') : t('login.submit')}
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}

export default LoginPage;
