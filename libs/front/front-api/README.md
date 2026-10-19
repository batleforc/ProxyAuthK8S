# front-api

`@proxy-auth-k8s/front-api`: the TypeScript client of the server API, generated
from `swagger.json` by [Hey API](https://heyapi.dev) (`openapi-ts.config.ts`).

- `pnpm --filter @proxy-auth-k8s/front-api generate` (or `task gen:api-client`)
  regenerates `src/lib`; never edit it by hand.
- The front consumes the package from source (`exports: ./src/index.ts`,
  `workspace:*`): there is no build step.
