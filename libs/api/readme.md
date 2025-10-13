# ProxyAuthK8s Api

- /api
  - Internal api of the reverse proxy
- /cluster/{ns}/{name}
  - /auth
    - /login => Authenticate user against api if enabled
    - /logout => Authenticate user against api if enabled
  - {.*} => To api
