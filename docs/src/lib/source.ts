import { docs } from 'fumadocs-mdx:collections/server';
import { loader } from 'fumadocs-core/source';
import { openapiPlugin } from 'fumadocs-openapi/server';
import { docsLoaderOptions } from '@weebo/operator-docs/site';

export const source = loader({
  source: docs.toFumadocsSource(),
  ...docsLoaderOptions,
  // The API reference pages (`task docs:api`) are OpenAPI pages: the kit has
  // no OpenAPI support, so its plugins are kept and this one added.
  plugins: [...docsLoaderOptions.plugins, openapiPlugin()],
});
