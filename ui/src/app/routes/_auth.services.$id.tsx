import { createFileRoute } from '@tanstack/react-router';
import { ServiceDetailPage } from '@/features/services/service-detail-page';

export const Route = createFileRoute('/_auth/services/$id')({
  component: ServiceDetailPage,
});
