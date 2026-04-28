import { createFileRoute } from '@tanstack/react-router';
import { IptablesPage } from '@/features/iptables/iptables-page';

export const Route = createFileRoute('/_auth/iptables')({
  component: IptablesPage,
});
