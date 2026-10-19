import { openapi } from '@/lib/openapi';
import { OpenAPIPageClient } from './api-page.client';

// fumadocs-openapi's generated MDX pages (see `content/docs/api/**/*.mdx`,
// produced by `scripts/generate-docs.ts`) still render `<APIPage document="..."
// operations={[...]} />` (the pre-v11 shape), but v11's renderer requires a
// `payload`/`preloaded` prop instead of resolving `document` itself, and
// there's nothing that wires one in. Bridge the gap here rather than
// hand-patching every generated file: this stays a Server Component (so
// `openapi.getSchemas()`, which touches the filesystem, never reaches the
// client bundle) and resolves `document` into the `payload` the real client
// component needs.
export async function APIPage({
  document,
  ...props
}: {
  document: string;
  [key: string]: unknown;
}) {
  const schemas = await openapi.getSchemas();
  const schema = schemas[document];
  if (!schema) {
    throw new Error(`[APIPage] no OpenAPI schema loaded for "${document}"`);
  }
  return <OpenAPIPageClient {...(props as Record<string, unknown>)} payload={{ bundled: schema.bundled }} />;
}
