# TODO — deferred code-health follow-ups

Larger refactors deferred during the desloppify code-health pass (strict score
88.2/100). Each was intentionally left for dedicated, well-tested work rather
than a rushed change. Roughly ordered by score impact / value.

## Architecture

- [x] **Split the `crd` schema crate from its runtime I/O.** Done: runtime
      methods (`is_reachable`, `get_client`, `to_kube_client`, `to_kubeconfig`,
      `get_oidc_conf`, plus `to_full_path`/`get_redirect_oidc_url` which also took
      `State`) moved into a new `libs/server/crd_runtime` crate behind the
      `ProxyKubeApiRuntime` extension trait; `impl ObjectRedis for ProxyKubeApi`
      moved into `common` (orphan rule). `crd` now depends only on
      serde/schemars/kube-derive/base64/regex — `crdgen` no longer links
      `common`/Redis/OIDC.

## Error handling & types

- [x] **Replace `Result<_, String>` with domain error enums.** Done (layered
      `thiserror`): `crd::certificate::CertError`, `crd::service::ServiceError`,
      `crd_runtime::ProxyRuntimeError` (wraps the two + reqwest/kube-client), and
      `api::model::user::UserAuthError`. HTTP paths still return the same opaque
      401/404/503 (detail only in logs / CRD status). The `tls.rs` forwarding
      path keeps its `Result<_, String>` boundary (adapts via `to_string`).

- [x] **Propagate CLI exit codes.** Done: every `CliCtx::handle_*` returns
      `Result<(), ProxyAuthK8sError>` (added an `InvalidUsage` variant for
      argument/usage failures); `run_cli` maps `Err` to `ExitCode::FAILURE`.
      Also fixed a pre-existing clap panic: `context`'s `--set` declared `-s`,
      colliding with the global `--server-url` `-s` (now long-only `--set`).

## Features

- [ ] **Let an admin use their kubeadm admin kubeconfig through the proxy
      (client-certificate auth).** Use case: an admin takes the `admin.conf`
      kubeadm generated (user authenticated by `client-certificate-data` /
      `client-key-data`, e.g. `CN=kubernetes-admin`, `O=kubeadm:cluster-admins`
      or `system:masters`), points `server:` at
      `/clusters/{ns}/{cluster}` and expects it to work. Today it can't:
      - the proxy never asks for a client certificate (the server is built
        with `with_no_client_auth()`, `libs/server/common/src/lib.rs`), so
        the request arrives with no credential and gets a 401 when validation
        is required;
      - even if it did, TLS ends at the proxy: the certificate can't be
        replayed upstream (the proxy doesn't have the private key), unlike a
        bearer token, which is forwarded as-is in `validate_against:
        Kubernetes` mode.

      Not to be confused with the existing `client_cert` on `ProxyKubeApi`,
      which is the proxy's own certificate towards the cluster.

      To do:
      - **TLS handshake:** request a client certificate as *optional* (OIDC
        and token clients must keep working). The handshake happens before
        routing, so it can't know the target cluster yet: accept the chain at
        the TLS level and do the real verification per request.
      - **Per-cluster verification:** a new CRD field (e.g. `client_ca`, often
        the cluster's own `ca.crt` with kubeadm) against which the presented
        chain is validated (signature, validity dates, `clientAuth` EKU), plus
        a switch to enable this auth mode per `ProxyKubeApi` (off by default).
        Note that Kubernetes has no revocation for client certificates; same
        limit here.
      - **Identity:** map the certificate like the apiserver does (`CN` →
        username, each `O` → group) into a `User`, so `allowed_resources`,
        the namespace rules and the `{{username}}`/`{{group}}` placeholders
        apply unchanged, and the audit log names the admin.
      - **Forward the identity upstream:** the proxy has to authenticate as
        itself and say who it acts for. Either impersonation (`Impersonate-User`
        / `Impersonate-Group` with a proxy credential that has the
        `impersonate` RBAC verb) or front-proxy / requestheader auth
        (`X-Remote-User` / `X-Remote-Group` with the front-proxy client cert).
        Both are currently stripped from client requests on purpose
        (`forwarded.rs`); they must only ever be set by the proxy from the
        verified certificate. Impersonating `system:masters` makes the proxy's
        credential cluster-admin-equivalent: document it and consider an
        allow-list of groups that may be impersonated.
      - **Ingress in front:** with TLS terminated by an ingress, the proxy never
        sees the certificate. Support either TLS passthrough, or a
        forwarded-certificate header (`ssl-client-cert` and similar) trusted
        only from a trusted proxy (same model as `TRUSTED_PROXY_COUNT`) and
        stripped from any other client.
      - **Kubeconfig:** document how to adapt `admin.conf` (proxy URL, proxy CA
        in `certificate-authority-data`, keep the user's certificate), or let
        the CLI rewrite it.
      - Helm chart values, tests (handshake with/without a certificate, wrong
        CA, expired certificate, spoofed headers, identity mapping), and a
        section in `.docs/content/docs/security.mdx`.

## Craft / structure

- [ ] **Bundle the redirect workers' argument block.** `standard_redirect`,
      `virtual_redirect`, and `upgrade` each take the same ~9 positional args and
      suppress `clippy::too_many_arguments`. Introduce a `RedirectContext` struct.
      Touches the security-sensitive forwarding hot path — cover with tests.

- [ ] **Rename the misspelled public CRD field `parametised` → `parametrised`.**
      This is a schema migration: add a `#[serde(alias = "parametised")]`, update
      the `x-kubernetes-validations` CEL rule strings that reference the field by
      name, the doc comments, the tests, and regenerate `crds.yaml`/`crds.mdx`
      (see the CRD-doc-regeneration notes). Also rename the internal helper
      `expand_parametised_patterns`.

## Testing

- [ ] **Add controller / reconcile coverage.** The reconcile loop has zero tests.
      The exponential-backoff retry-delay calculation is extractable and pure —
      pull it out and unit-test it. Broader reconcile coverage needs an
      envtest-tier integration test (real apiserver + Redis), like the existing
      envtest suite. Test health is the top mechanical drag (50% strict).

- [ ] **Test the Kubernetes auth mode.** `User::auth_against_kubernetes` (the
      `SelfSubjectReview` path) is uncovered; needs an envtest / mock-apiserver
      test. The OIDC mode is already wiremock-tested.

- [ ] **Broaden CLI test coverage.** Only a few of the CLI crate's files have
      tests. Add tests for the remaining pure output/mapping logic (e.g. the
      `KubeList`/`TableRow` renderers, `detect_kubeconfig_path`,
      `to_tracing_verbose_level`, and `helper::secure_write` file permissions).

## Dependencies (maintainer decisions)

- [ ] Migrate off the deprecated `serde_yaml` (0.9.34+deprecated) — preserve exact
      CRD YAML output.
- [ ] Resolve the dual `reqwest` build (0.12 via the OpenTelemetry stack + 0.13
      pinned) once the otel ecosystem moves to 0.13.
- [ ] Revisit the alpha `oauth2-reqwest` (0.1.0-alpha.3) dependency on the core
      `common` lib.
- [x] `task audit` is wired up (`cargo audit` + `yarn npm audit` + `trivy`).
      Fixed 2026-08-08: bumped `rustls-webpki` 0.103.10 → 0.103.13
      (RUSTSEC-2026-0098/0099/0104) and `quinn-proto` 0.11.14 → 0.11.16
      (RUSTSEC-2026-0185, high severity) via `cargo update -p <crate>` — both
      were pure `Cargo.lock` bumps within the existing semver constraints, no
      `Cargo.toml` change needed. Verified: workspace build, clippy, and the
      full 241-test suite (nextest) all green after the bump.
      **Still open:** `rsa` (RUSTSEC-2023-0071, Marvin timing sidechannel
      attack) has **no fixed upgrade available upstream** — nothing to do
      here until `rsa` ships one; re-check periodically.
