import { createDocsPage } from '@weebo/operator-docs/site';
import { getMDXComponents } from '@/components/mdx';
import { site } from '@/lib/shared';
import { source } from '@/lib/source';

const docsPage = createDocsPage({ source, site, getMDXComponents });

export default docsPage.Page;
export const { generateStaticParams, generateMetadata } = docsPage;
