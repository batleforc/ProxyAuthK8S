# ProxyAuthK8s Api

HTTP surface of the server (actix-web + utoipa). The full, generated reference is
served by the server at `/api/docs` (Scalar) and published in the docs site
(`.docs/content/docs/api`, regenerated with `task gen:swagger-file && task gen:docs:api`).

- `/management`
  - `GET /health` => liveness (always 200 while the process serves HTTP)
  - `GET /ready` => readiness (200 when Redis answers a PING, 503 otherwise)
- `/api/v1`
  - `GET /clusters` => clusters visible to the authenticated user (dashboard)
- `/clusters/{ns}/{cluster}`
  - `GET /auth/login` => start the per-cluster OIDC login (redirect to the provider)
  - `GET /auth/callback` => OIDC callback, then redirect to the dashboard / kubectl plugin
  - `GET /.well-known/oauth-authorization-server` => RFC 8414 metadata of the
    per-cluster OAuth 2.0 authorization server (when discovery is enabled)
  - `GET /oauth/authorize`, `GET /oauth/callback`, `POST /oauth/token`,
    `GET /oauth/jwks` => mediated OAuth 2.0 authorization server (PKCE S256,
    loopback redirect URIs only)
  - `{path}` (GET/POST/PUT/PATCH/DELETE, plus upgrade for exec/attach/port-forward)
    => authenticated, authorized reverse proxy to the cluster's apiserver

## Authentication on the proxy path

Each `ProxyKubeApi` chooses how the caller's bearer token is validated
(`auth_config.validate_against`):

- `OidcProvider`: `/userinfo` on the cluster's OIDC provider, then audience
  checks (introspection, else JWT `aud`). Discovery documents and validated
  tokens are cached briefly (see `common::oidc_cache`).
- `Kubernetes`: a `SelfSubjectReview` against the target cluster with the token.

The resolved user (name, email, groups) then goes through the cluster's
authorization rules (`proxy_group`, `allowed_resources`, namespace rules,
quotas / fail2login). See `.docs/content/docs/security.mdx`.
