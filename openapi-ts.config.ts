import { defineConfig } from '@hey-api/openapi-ts';

export default defineConfig({
  input: './swagger.json',
  output: 'libs/front-api/src/lib',
  plugins: ['@hey-api/client-axios'],
});
