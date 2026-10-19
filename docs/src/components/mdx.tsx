import { docsMdxComponents } from '@weebo/operator-docs/site';
import type { MDXComponents } from 'mdx/types';
import { APIPage } from '@/components/api-page';
import { Mermaid } from '@/components/mdx/mermaid';
import { ProxyKubeApiGenerator } from '@/components/proxykubeapi-generator.server';

const kit = docsMdxComponents();

// The kit's CrdForm does not render ProxyKubeApi's serde `oneOf` unions (RFC
// 0005, decision 2): the generated CRD page gets ProxyKubeApiGenerator instead.
function CrdForm({ kind }: { kind: string }) {
  return kind === 'ProxyKubeApi' ? <ProxyKubeApiGenerator /> : <kit.CrdForm kind={kind} />;
}

export function getMDXComponents(components?: MDXComponents) {
  return {
    ...kit,
    CrdForm,
    APIPage,
    // fumadocs-openapi >= 12 generates `<OpenAPIPage document=... />`.
    OpenAPIPage: APIPage,
    Mermaid,
    ProxyKubeApiGenerator,
    ...components,
  } satisfies MDXComponents;
}

export const useMDXComponents = getMDXComponents;

declare global {
  type MDXProvidedComponents = ReturnType<typeof getMDXComponents>;
}
