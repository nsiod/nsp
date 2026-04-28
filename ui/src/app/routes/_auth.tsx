import { createFileRoute, redirect } from '@tanstack/react-router';
import { Layout } from '@/shared/components/layout';
import { authStore } from '@/shared/stores/auth';

export const Route = createFileRoute('/_auth')({
  beforeLoad: () => {
    if (!authStore.isAuthenticated()) {
      throw redirect({ to: '/login' });
    }
  },
  component: Layout,
});
