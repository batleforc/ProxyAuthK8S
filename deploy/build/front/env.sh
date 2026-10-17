#!/bin/sh
# Fill the PROXYAUTHK8S_FRONT_* markers the build took from .env.production
# with the container's variables of the same name. An unset variable becomes
# empty, so the front falls back to its default (`/api` for the API base URL).
set -eu
# The markers of .env.production.
for key in PROXYAUTHK8S_FRONT_API_BASE_URL PROXYAUTHK8S_FRONT_OIDC_ISSUER_URL \
    PROXYAUTHK8S_FRONT_OIDC_CLIENT_ID PROXYAUTHK8S_FRONT_OIDC_SCOPE \
    PROXYAUTHK8S_FRONT_OIDC_SILENT_REFRESH; do
    # `|`, `&` and `\` are special in the sed replacement below.
    value=$(printenv "$key" | sed 's/[|&\\]/\\&/g' || true)
    echo "$key=$(printenv "$key" || true)"
    find /app -type f \( -name '*.js' -o -name '*.css' \) -exec sed -i "s|${key}|${value}|g" '{}' +
done
