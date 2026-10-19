import { ogRoute } from '@weebo/operator-docs/routes';
import { site } from '@/lib/shared';
import { source } from '@/lib/source';

export const revalidate = false;
export const dynamic = 'force-static';

export const { GET, generateStaticParams } = ogRoute(source, site);
