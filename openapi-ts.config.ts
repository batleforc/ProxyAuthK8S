import { defineConfig } from '@hey-api/openapi-ts';

export default defineConfig({
  input: './swagger.json',
  output: 'libs/front/front-api/src/lib',
  plugins: ['@hey-api/client-axios'],
  parser: {
    filters: {
      tags: {
        exclude: ['proxy_clusters', 'health'],
      },
    },
  },
});
