import defaultMdxComponents from 'fumadocs-ui/mdx';
import type { MDXComponents } from 'mdx/types';
import { APIPage } from '@/components/api-page';
import { Mermaid } from '@/components/mdx/mermaid';
import { ProxyKubeApiGenerator } from '@/components/proxykubeapi-generator.server';

export function getMDXComponents(components?: MDXComponents): MDXComponents {
  return {
    ...defaultMdxComponents,
    APIPage,
    // fumadocs-openapi >= 12 generates `<OpenAPIPage document=... />`.
    OpenAPIPage: APIPage,
    Mermaid,
    ProxyKubeApiGenerator,
    ...components,
  };
}
