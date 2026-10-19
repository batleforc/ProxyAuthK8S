import { llmsPageRoute } from '@weebo/operator-docs/routes';
import { source } from '@/lib/source';

export const revalidate = false;
export const dynamic = 'force-static';

export const { GET, generateStaticParams } = llmsPageRoute(source);
