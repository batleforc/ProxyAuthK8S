# ProxyAuthK8s Api

- /api
  - Internal api of the reverse proxy
  - /cluster
    - GET / => List clusters
- /cluster/{ns}/{name}
  - /auth
    - /login => Authenticate user against api if enabled
    - /callback => Oidc callback endpoint
    - /refresh => Refresh token endpoint
    - /logout => Authenticate user against api if enabled
  - {.*} => To api

## Oidc for cluster auth

4 endpoints to handle the oidc flow for user authentication dedicated to each cluster:

- /auth/login => Redirect the user to the oidc provider
- /auth/callback => Handle the callback from the oidc provider
- /auth/refresh => Refresh the token using the refresh token
- /auth/logout => Logout the user
- All of that with the [openidconnect-lib](https://docs.rs/openidconnect/latest/openidconnect/#asynchronous-api)

## Oidc for the main api

Two auth workflows are possible:

- Full Frontend flow where the frontend handles the oidc flow and sends the token to the api that will validate it
- Backend flow where the api handles the oidc flow and then sends the token to the frontend
