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

- [x] **`kubectl proxyauth login <cluster>` writes the kubeconfig context.** Done
      (`libs/cli/cli/src/login/kubeconfig.rs`): upserts cluster `<ns>-<cluster>`, user
      `<ns>-<cluster>-proxyauth` (exec → `get-token`, `interactiveMode: Never`, which
      kubectl requires for the v1 exec API — the dashboard's kubeconfig was missing it too)
      and context `<ns>-<cluster>-context`, then sets it current. Unit-tested and
      checked against real `kubectl`.

- [x] **`--certificate-authority` on `login`.** Done: the PEM is validated, saved per server
      (`certificate_authority_data` in `proxyauth_config.yaml`), trusted on top of the system
      roots by every CLI call to that server, and written as `certificate-authority-data`
      in the kubeconfig. Possible follow-up: an `oc login`-style trust-on-first-use prompt
      showing the fingerprint of the certificate the server presents.
- [x] **Multi-file `$KUBECONFIG`.** Done: the CLI reads the merged view (first file wins,
      missing files skipped) and writes only to the first existing file, re-read from its
      own content so other files' entries are never copied into it.

## Bugs found while writing the docs (2026-10-05)

- [x] **Helm chart: Ingress targeted non-existent services.** Fixed: uses the
      `proxyauthk8s.front.name`/`back.name` helpers and `.Values.front.port` (also in
      `svc.front.yaml`). Quick-start workaround removed.
- [x] **Helm chart: wrong default images.** Fixed: `ghcr.io/batleforc/proxyauthk8s/server|front`,
      tag defaults to `.Chart.AppVersion` (now `v0.1.9`), and `cog.toml` bumps
      `appVersion` on release.
- [x] **Helm chart: no front OIDC config with `oidc.source: secret`.** Fixed: the front
      reads issuer/client id/scopes from the same secret (`secretKeyRef`, never the
      client secret).
- [x] **`kubectl proxyauth ctx` didn't exist.** Fixed: `ctx` alias on `context`; the
      callback page now sets the real context name (`<ns>-<cluster>-context`) and passes
      `-n <ns>` to `login`.
- [x] **Plugin kubeconfig passed `-n <ns>` / `-s <url>` as one argv.** Confirmed (clap
      read `' x-example'`); fixed with `--namespace=` / `--server-url=`.

## Craft / structure

- [x] **Bundle the redirect workers' argument block.** Done 2026-08-29:
      `RedirectContext` lives in
      `libs/server/api/src/cluster/redirect/kube_redirect/context.rs` and carries
      the nine shared values plus `base_url`/`upstream_path`. The three
      signatures are now `standard_redirect(ctx)`, `upgrade_redirect(ctx)` and
      `virtual_redirect(ctx, registry, plan)`, and all three
      `#[allow(clippy::too_many_arguments)]` are gone. Net −30 lines.
      *Kept deliberately low-risk:* each worker destructures the context into
      the same local bindings on its first line, so every worker **body is
      byte-identical** to before — the diff is signatures and imports only. The
      `#[instrument]` spans were rewritten as `skip_all` + explicit fields that
      reproduce exactly what the old positional signatures recorded (`method`,
      `peer_addr`, `proxy`, `url_to_call`), so nothing observable changed
      either.
      *What this buys:* the call sites were nine positional arguments, four of
      them `String`/`Option<..>`-shaped alike — transposing two would have
      compiled and silently forwarded the wrong thing on the auth-checked hot
      path. They are now named struct fields, so that class of mistake is a
      compile error.
      *Tests:* the URL assembly the dispatcher used to do inline moved into a
      pure `join_upstream_url(base_url, upstream_path, query_string)`, with six
      unit tests covering the no-query case, the single `?`, a query forwarded
      byte-for-byte (re-encoding a `fieldSelector`/`labelSelector` would change
      what the cluster's own RBAC evaluates), the empty path at the cluster
      root, a path that legitimately repeats `clusters/` or holds a `%2F`, and
      the absence of a trailing `?`. The end-to-end behaviour is covered by the
      existing `proxy_redirect` / `proxy_security` / `virtual_api` tiers, which
      pass unchanged.
      **Still open:** `list_fallback`'s three `too_many_arguments` suppressions
      (`list_projects_filtered`, `resolve_unresolved`, and one more in
      `resolve.rs`). They take a *borrowed*, overlapping-but-different block, and
      the workers destructure the context away before calling them, so folding
      those in means keeping `ctx` intact through the workers — a much larger
      change than this one, and a separate decision.
      **Also noted, not acted on:** these spans record the whole `ProxyKubeApi`
      via `Debug` on every proxied request, which is a lot of span payload for
      data the audit context already carries as ns/cluster. Preserved as-is here
      because this was a no-behaviour-change refactor; worth trimming on its own.
      *Merged 2026-10-09 with main's parallel version:* main's port policy is
      passed alongside, so the upgrade worker is `upgrade_redirect(ctx, port_policy)`,
      and spans/`info` logs record the URL without its query
      (`RedirectContext::url_without_query`).
- [x] ~~Rename `parametised` → `parametrised`.~~ Dropped: `parametised` is the intended
      spelling of the field and of `ParametisedRule`; keep it.

- [x] **Redact the OIDC `client_secret` from `Debug`.** Done: `OidcProvider` has a
      hand-written `Debug` (destructured, so a new field forces a decision) printing
      `client_secret: Some("<redacted>")`. A test formats a whole `ProxyKubeApi` with an
      OIDC secret and an inline client cert/key and checks nothing secret comes out.

## Testing

- [x] **Add controller / reconcile coverage.** Done 2026-08-29: 24 tests across
      three tiers. `libs/server/controller/src/proxy_kube_api/reconcile.rs` and
      `cleanup.rs` are now at 100% line coverage, `proxy_kube_api/mod.rs` at 88%,
      `error.rs` at 100%.
      *Unit (10, no dependencies):* the backoff calculation was extracted as
      `reconcile::retry_delay_seconds(attempts)` with the three requeue
      constants lifted to module scope — tested for the base delay, the
      doubling, the cap at the success interval (a broken cluster must never be
      polled *less* often than a healthy one), monotonicity, and no overflow up
      to `u32::MAX`. Plus the leader gate (a follower requeues at ~lease TTL and
      touches neither Redis nor the apiserver), the error policy's two delays,
      the namespace-less resource rejection, and `ControllerError`'s metric
      label / Redis conversion.
      *Fast integration (9, real Redis + wiremock target cluster, in
      `libs/server/controller/tests/reconcile.rs`):* the cache and index writes,
      the success and backoff requeues, three consecutive failures doubling then
      a recovery clearing the retry counter, an invalid configuration being
      reported without probing, a failed cache write surfacing as an error
      rather than a false success, and the cleanup path (including idempotency
      and an unreachable Redis not wedging the finalizer). These pre-set the
      status the reconcile computes so the "patch only if it changed" branch is
      not taken and no apiserver is needed.
      *envtest (5, real apiserver + Redis, in
      `libs/server/api/tests/envtest_reconcile.rs`):* the status actually landing
      on the resource through the `status` subresource, the error status, the
      no-repatch optimization asserted on `resourceVersion` (only a real
      apiserver maintains it), the full finalizer lifecycle through
      `main_reconcile_proxy_kube_api` (add, apply, delete, cleanup, release),
      and a follower writing neither status nor finalizer. It lives in the `api`
      crate — where the envtest harness already is — with `controller` as an
      `api` dev-dependency, rather than duplicating that ~460-line harness.
      *Also fixed along the way:* `.github/workflows/back-envtest.yaml` ran
      `--test envtest`, a single binary, so `envtest_list_fallback.rs` was
      **never running in CI**; it now uses the same `binary(/^envtest/)`
      selector as `task envtest:run`, and the job gained the Redis service the
      reconcile tier needs.
      **Still open:** `controller/src/lib.rs` (`run` and `run_leader_election`)
      is still at 0% — it is the wiring that needs a live cluster and a real
      Lease, not reconcile logic.
- [x] **Add controller / reconcile coverage.** Done 2026-10-05: unit tests
      (backoff, leader demotion) + envtest-tier `api/tests/envtest_reconcile.rs`
      running the real controller (create → Redis + status + finalizer,
      disable, unreachable target, delete → cleanup; follower writes nothing).
      Found and fixed: cleanup left the `requeue_retry:<id>` counter in Redis.

- [x] **Test the Kubernetes auth mode.** Done 2026-08-29: 10 tests in
      `libs/server/api/tests/kubernetes_user.rs`, mirroring how `oidc_user.rs`
      covers the OIDC mode. **No envtest was needed** — the proxy fixture
      already uses `Service::ExternalService` + `CertSource::Insecure`, so
      neither `url_to_call` nor `get_cert` touches `state.client` and
      `to_kube_client` builds a plain-HTTP client aimed straight at a wiremock
      server standing in for the target apiserver (`POST
      /apis/authentication.k8s.io/v1/selfsubjectreviews`). No Redis either.
      Covered: the resolved user (username, groups, `email` out of the `extra`
      map), the review being sent *as the caller* (bearer token asserted, empty
      body), a missing `status`/`userInfo` resolving to an empty user rather
      than an error, the `extra.email` variants (absent / other key / empty
      list / first-of-several), 401 and 500 and an unreachable cluster all
      mapping to `UserAuthError::SelfSubjectReview`, a malformed cluster URL
      mapping to `UserAuthError::Runtime`, and the `validate_against:
      Kubernetes` dispatch in `get_user_info_with_proxy` (plus the `auth_config:
      None` short-circuit, asserted to make no upstream call).
      `model/user.rs` line coverage from the two user-auth test files alone:
      16.67% -> 44.05%. Added `harness::kubernetes_auth_config()` and listed the
      file in the `test:integration` task.
      **Still open:** `User::from_request` (the actix extractor) and
      `get_user_info`'s Redis lookup remain uncovered here — they are exercised
      end-to-end by the proxy tests instead.
- [x] **Test the Kubernetes auth mode.** Done 2026-10-05:
      `api/tests/envtest_kube_auth.rs` (ServiceAccount token via TokenRequest →
      identity + groups; unknown / deleted-SA token rejected; proxy forwards a
      valid token, 401 without detail otherwise).

- [x] **Broaden CLI test coverage.** Done 2026-08-29: 46 unit tests added to the
      `cli` crate, all test-only (no production code touched). Covered the pure
      output/mapping logic that was listed — the generic `KubeList`/`TableRow`
      renderer (envelope, JSON/YAML round-trips, table layout, empty list,
      `to_output` dispatch), `detect_kubeconfig_path` (flag > `KUBECONFIG` >
      `$HOME/.kube/config`, and the `None` case), `to_tracing_verbose_level`
      (incl. saturation past `-vv`), and `helper::secure_write` (created `0600`,
      and an existing `0644` file *tightened* to `0600`) — plus the three
      remaining `TableRow` impls (`GetClusterOutput`, `GetContextOutput`,
      `GetOutput`), the `error/convert.rs` mappings, and the `CliConfig`
      YAML/lookup helpers (`get_cluster_config_by_url` telling a wrong host from
      a wrong cluster). Line coverage for the crate: 10.74% -> 48.39%.
      *Note:* the `detect_kubeconfig_path` tests mutate the process
      environment. Nextest gives one process per test, but they also take a
      `Mutex` and restore the previous values so they stay correct under a plain
      threaded `cargo test`.
      **Still open:** the keyring (`keyring_store.rs`), HTTP (`remote.rs`,
      `login/`), and filesystem-writing (`ctx/build.rs`, `config_cmd/{clear,
      set_def}.rs`) paths are still at 0% — they need fakes or a wiremock tier,
      not more pure-logic tests. (`COVERAGE_MIN` was raised to 62 later the same
      day, once a live Redis made an honest workspace measurement possible —
      see the controller item above.)
- [x] **Broaden CLI test coverage.** Done 2026-10-05: 28 tests (output
      renderers, `secure_write` permissions, ctx helpers, login helpers). Was: Only a few of the CLI crate's files have
      tests. Add tests for the remaining pure output/mapping logic (e.g. the
      `KubeList`/`TableRow` renderers, `detect_kubeconfig_path`,
      `to_tracing_verbose_level`, and `helper::secure_write` file permissions).

## Dependencies (maintainer decisions)

- [x] Migrated off `serde_yaml` to `serde_yaml_ng` 0.10 (same libyaml
      backend; CRD, kubeconfig and `-f yaml` output byte-identical; no longer
      in Cargo.lock).
      (Merged 2026-10-09: main's migration supersedes the branch's earlier
      "stay on `serde_yaml`" decision.)

- [x] Resolve the dual `reqwest` build (0.12 via the OpenTelemetry stack + 0.13
      pinned). Fixed 2026-08-29: no otel upgrade was needed — the 0.12 copy came
      from `opentelemetry-otlp`'s *default* features (`http-proto` +
      `reqwest-blocking-client`), which the tonic-only exporters in
      `libs/server/trace` never used. Set `default-features = false` on the
      workspace dep and listed the features explicitly (`grpc-tonic`, `trace`,
      `metrics`, `logs`, `internal-logs`, `serialize`). `reqwest` 0.12 is gone
      from `Cargo.lock`; only the pinned 0.13 remains. Verified: workspace
      check, clippy `--all-targets`, and the full 264-test suite all green.
- [ ] Revisit the alpha `oauth2-reqwest` (0.1.0-alpha.3) dependency on the core
      `common` lib. Reviewed 2026-08-29 — **decision: keep it**, but it is worth
      re-checking when upstream moves.
      *Why it is there:* `openidconnect` 4.0.1's `reqwest` feature routes to
      `oauth2/reqwest`, and `oauth2` 5.0.0 pins `reqwest = "0.12"`. That is why
      the workspace declares `openidconnect` with `default-features = false`.
      `oauth2-reqwest` is the upstream split-out adapter (same author,
      ramosbugs/oauth2-rs) providing the same `AsyncHttpClient` over reqwest
      0.13. So the two lines are load-bearing together: re-enabling
      openidconnect's default features would drag reqwest 0.12 back in and undo
      the dual-`reqwest` fix above.
      *Risk assessment:* 196 lines, MIT, zero `unsafe`, no new transitive deps
      (`oauth2` and `reqwest` are already in the graph via `openidconnect`). The
      pre-release version reflects that the crate is new — it was split out so
      reqwest 0.13 support would not force a semver-major `oauth2` — not that
      the code is in flux. alpha.3 is the newest published version. We use
      exactly one item, `ReqwestClient`, at `libs/server/common/src/oidc_conf.rs`;
      the `blocking` half is not compiled.
      *Alternatives considered and rejected:* (a) vendor the ~40-line async
      `AsyncHttpClient` impl into `common` — removes the alpha from the graph but
      makes us track `oauth2`'s trait signature by hand, for code we would copy
      verbatim from upstream anyway; (b) go back to reqwest 0.12 and
      openidconnect's built-in client — reintroduces the dual build.
      *Re-check when:* `oauth2` 6 ships (expected to fold reqwest 0.13 in
      directly, making this crate unnecessary), or `oauth2-reqwest` 0.1.0 stable
      lands. Optional hardening if the alpha ever churns: pin exactly
      (`=0.1.0-alpha.3`) so a `cargo update` cannot pull an alpha.4 into the auth
      path unnoticed.
- [x] `task audit` is wired up (`cargo audit` + `yarn npm audit` + `trivy`).
      Fixed 2026-08-08: bumped `rustls-webpki` 0.103.10 → 0.103.13
      (RUSTSEC-2026-0098/0099/0104) and `quinn-proto` 0.11.14 → 0.11.16
      (RUSTSEC-2026-0185, high severity) via `cargo update -p <crate>` — both
      were pure `Cargo.lock` bumps within the existing semver constraints, no
      `Cargo.toml` change needed. Verified: workspace build, clippy, and the
      full 241-test suite (nextest) all green after the bump.
      Also 2026-08-29: bumped `h2` 0.4.15 → 0.4.19 (RUSTSEC-2026-0258, unbounded
      empty DATA frames) — again a pure `Cargo.lock` bump. Verified: clippy and
      the full 264-test suite green.
      **Still open:** `rsa` (RUSTSEC-2023-0071, Marvin timing sidechannel
      attack) has **no fixed upgrade available upstream** — nothing to do
      here until `rsa` ships one; re-check periodically.
      **Also still open:** the *second* copy of `h2`, 0.3.27, pulled by
      `actix-http` (`h2 ^0.3.26`), is hit by the same RUSTSEC-2026-0258. The
      advisory's fix is `>=0.4.16` and 0.3.27 is the last release on the 0.3
      line, so there is no in-semver bump — it clears only when actix-web moves
      to `h2` 0.4 (actix-web 5) or upstream backports a 0.3.28. The other lever
      would be dropping actix-web's default `http2` feature, which is a
      behaviour change for proxy clients and so a deliberate maintainer call,
      not an audit fix.

## Production-readiness review (2026-10-05)

Findings of the code-quality / prod-readiness / docs-coverage review. Verdict:
alpha / early beta (0.1.9) — app-level security is careful, operations are not
there yet.

### Blockers (must fix before prod)

- [x] **Helm: leader election can never succeed.** Fixed: namespaced
      `Role`/`RoleBinding` granting `get`/`create`/`patch` on `leases` in the
      release namespace (`rbac.operator.yaml`); `LEASE_NAMESPACE` and `POD_NAME`
      set via the downward API (`dp.back.yaml`). Was: The operator ClusterRole has
      no `coordination.k8s.io/leases` rule, so `try_acquire_or_renew` fails
      forever, `is_leader` stays false, reconcile never runs, Redis is never
      filled and every `/clusters/...` call is a 404. `LEASE_NAMESPACE` is not
      set by the chart either (falls back to `default`).
- [x] **No timeouts on outbound calls.** Fixed: shared IdP client with 5s
      connect / 10s total; upstream apiserver client and upgrade (exec /
      port-forward) connections get a 10s connect + TLS-handshake bound only,
      so long-lived streams are not cut. Was: The upstream client
      (`kube_redirect/upstream.rs`) and the OIDC HTTP client
      (`common/src/oidc_conf.rs`) set no connect/request timeout: a hung
      apiserver or IdP pins workers.
- [x] **2–3 IdP round-trips per authenticated request, nothing cached.**
      Fixed (`common::oidc_cache`): discovery + introspection endpoint cached
      per issuer (5 min); validated tokens cached by SHA-256 key scoped to
      issuer/client/audience for `OIDC_TOKEN_CACHE_TTL` (30s, `0` disables,
      capped by JWT `exp`); one pooled IdP client. Caches are not populated
      under the `test-util` feature (wiremock reuses ports). **Still open:** a
      new upstream reqwest client + TLS config per proxied request (see
      "per-cluster upstream client pool"). Was:
      `get_user_info_from_oidc_token` re-runs OIDC discovery, then userinfo
      (and maybe introspection) on every call; a fresh reqwest client + TLS
      config is also built per upstream request.
- [x] **Shallow health probe.** Fixed: `/management/health` stays a
      dependency-free liveness (logs at debug); new `/management/ready` PINGs
      Redis (2s) → 200/503, wired as the chart's readinessProbe. Was: `/management/health` always answers 200 (no
      Redis check) and is used for both liveness and readiness, so a pod with
      Redis down stays in rotation answering 503. It also logs at `info` on
      every probe.
- [x] **Release pipeline.** `build.yaml` triggers on `deploy/build/**`,
      `Cargo.lock`, `yarn.lock`; release images built natively per arch
      (`ubuntu-24.04` / `ubuntu-24.04-arm`), pushed by digest and merged into a
      multi-arch index (`linux/amd64` + `linux/arm64`) with provenance + SBOM
      attestations, cosign keyless signature (`--recursive`) and the SBOMs
      attached to the release; every action pinned by SHA; otel pinned;
      `task lint:actions` (actionlint) added. Fixed on the way: the retired
      `macos-13` runner (would have blocked every release) and the krew
      placeholder check that could not fail under `set -e`. **To verify on the
      first real tag.**
- [x] **Chart defaults.** Fixed: back CPU limit `1000m`, back/front
      `replicas: 2` with PDBs on, `NOTES.txt` warns about `oidc.source=env`,
      a disabled NetworkPolicy and `< 2` replicas (NetworkPolicy stays off by
      default: the chart can't guess where Redis / target apiservers are). Was: Back CPU limit `100m` (too low for a streaming
      proxy), `replicas: 1`, PDB/HPA/NetworkPolicy off; with
      `oidc.source=env` the client secret is plain text in the pod spec.

### To fix next (security / ops)

- [x] CORS: the chart now sets `CORS_ALLOWED_ORIGINS` to the ingress origin
      (`back.corsAllowedOrigins` to override). The binary default stays
      permissive for backward compatibility.
- [x] Operator RBAC: secrets/configmaps/services reduced to `get` (the code
      never lists/watches them; configmaps were missing, which broke
      `CertSource::ConfigMap`); `rbac.referencedNamespaces` swaps the
      cluster-wide grant for per-namespace Roles. Back Containerfile now runs
      as `USER 1000:1000`.
- [x] Logging: upstream URLs path-only at `info`; `LOG_FORMAT=json`
      (chart `back.logFormat`); stdout level now follows `RUST_LOG`.
- [ ] Startup: the controller returns `ControllerError::CrdUnavailable`
      instead of panicking (server exits non-zero → CrashLoopBackOff);
      `REDIS_URL`/`LEASE_NAMESPACE` defaults now log a warning — done.
      **Still open:** boot still fails hard if OIDC discovery fails (decide:
      retry with backoff vs. fail fast).
- [x] Shutdown: `SERVER_SHUTDOWN_TIMEOUT` (default 30s); the chart sets it
      from `back.shutdownTimeoutSeconds` and the pod grace period to +15s.
- [ ] Known advisories: `rsa` (RUSTSEC-2023-0071, no fix) and `h2 0.3` via
      actix-http (RUSTSEC-2026-0258, no 0.3 fix).
- [ ] opentelemetry 0.33 blocked until tracing-actix-web ships
      `opentelemetry_0_33` (actix/actix-extras#838); TypeScript 7 blocked by
      typescript-eslint / vue-tsc; ESLint 10 in `.docs` blocked by
      eslint-config-next (vercel/next.js#89764).
- [x] Chart signing + `values.schema.json`: charts are pushed with native
      `helm push` and signed with cosign keyless; the schema rejects unknown
      keys / bad types. Per-cluster upstream client pool:
      `common::upstream_cache` (key = ns/name + hash of uid/cert/client_cert,
      TTL `PROXY_UPSTREAM_CLIENT_TTL` default 300s, evicted on proxy deletion)
      — ~21× more req/s locally, 1 upstream connection instead of one per
      request, no Kubernetes read per request. Also fixed the release notes
      template (wrong chart OCI path/version, CLI section) and the krew
      manifest usage. (Also done earlier: Prometheus metrics, multi-arch,
      `POD_NAME`.)

### Code-quality weaknesses

- [x] `crd_runtime` tests: 20 unit + 13 wiremock/TCP + 3 envtest. Fixed:
      certs stored as raw PEM in a Secret (cert-manager, `kubectl create
      secret tls`) could not be read (always base64-decoded twice); a CA with
      no PEM block is now rejected (`CaWithoutPem`) instead of silently
      falling back to the default roots.
- [x] Token validation has direct tests: `api/tests/oidc_token_validation.rs`
      (21 tests: valid, expired, malformed userinfo, inactive introspection,
      wrong/missing audience, azp opt-in, wrong issuer, HTTP 401 without
      detail) + 5 more `token_audience` unit tests.
- [x] `unwrap`/`expect`/`panic!` on startup paths: `trace` returns a
      `TraceError` and falls back to stdout-only logging when the OTLP exporter
      can't be built; controller and CLI context fixed.
- [x] `load_discovery_enabled_proxy` now returns a small
      `DiscoveryGateError`; `ReadMoreError` is a `thiserror` enum. Clippy is
      warning-free.
- [x] Very long functions: CLI ones split; `upgrade_redirect` 209 → 51
      lines (`upgrade.rs` split into `upgrade/{connect,response,tunnel}.rs`,
      refusals as an `UpgradeRefusal` enum), `virtual_redirect` 213 → 57
      (`Forward` + `VirtualFailure`); no helper over 60 lines, 40 new unit tests.
- [x] CLI keyring access folded into `keyring_entry` + read/write/delete
      helpers (same service/user naming, stored tokens stay readable).
- [x] Configuration centralised in `common::config::Config` (`from_env` →
      pure `from_lookup`, installed once from `main`, lazily loaded in tests);
      every server env read goes through it, except
      `PROXYAUTH_ALLOW_CROSS_NS_CERT`, still read by `crd` (schema crate must
      not depend on `common`; same parser shared). Unparsable values now log
      a warning before falling back to the default.
- [x] `common/src/lib.rs` split into `state.rs`, `state/redis.rs`,
      `server_config.rs`, `error.rs`, `config.rs`; `lib.rs` only re-exports
      (all public paths unchanged).
- [x] Front: views split into components (`CliView` 1294 → 175 lines,
      `ClusterCallbackView` 1247 → 236, `HomeLoggedout` 1109 → 22,
      `ClusterNoSSOView` 744 → 255, `nav.vue` 781 → 237), shared `BrandLogo`,
      `SectionCard`, cluster cards, `useCopyToClipboard`, kubeconfig/CLI
      command builders in `utils/`; 50 vitest tests; `/about` is a real public
      page. `vue-tsc` typecheck is now clean (tsconfig `module: ESNext` +
      `moduleResolution: bundler`, static icons from `@maz-ui/icons/static`)
      and enforced in CI (`front-lint.yaml` job `check`). Fixed on the way:
      toasts used the non-existent `duration` option (now `timeout`), badges
      the non-existent `danger` colour (now `destructive`). Only verified by
      markup/computed-style snapshots: eyeball responsive (480/768 px), hover
      and dark-mode nav once in a browser.

- [ ] **Decide: `CertSource::Insecure(true)`** is documented as "do not use
      TLS" but actually verifies the upstream with the system trust store.
      Either fix the doc/CRD description, or make it really skip verification
      (security-sensitive — prefer renaming to something like `SystemRoots`).
- [ ] **Decide:** `is_reachable` returns `Err(Http)` (not `Ok(false)`) on a
      closed port; tests pin the current behaviour.

### Docs — missing or stale

- [x] **Stale API reference:** regenerated (`swagger.json`, `api/` pages via
      the new `task gen:docs:api`, which keeps the hand-written `index.mdx`,
      `client_api`, `front-api`). Was: committed `swagger.json` / `.docs/swagger.json`
      only has 5 paths; the OAuth AS endpoints (well-known, authorize,
      callback, token, jwks) and their schemas are missing. Regenerate the
      spec, the `api/` pages and `libs/cli/client_api`.
- [x] **Undocumented server env vars:** new page
      `.docs/content/docs/configuration.mdx`. Was: `SERVER_PORT`, `SERVER_HTTPS`,
      `SERVER_CERT_PATH`, `SERVER_KEY_PATH` (HTTPS mode undocumented),
      `API_CLUSTER_OIDC_BASE_REDIRECT_URL`, `API_CLUSTER_OIDC_FRONT_REDIRECT_URL`,
      `OIDC_REDIRECT_URL`, `LEASE_NAMESPACE`, `HOSTNAME`, `REDIS_CLUSTER`,
      `POD_NAME`, `OTEL_EXPORTER_OTLP_ENDPOINT`.
- [x] **Undocumented front env vars:** documented (dev `VITE_*` and image
      `PROXYAUTHK8S_FRONT_*` runtime substitution). Was: `VITE_OIDC_ISSUER_URL`,
      `VITE_OIDC_CLIENT_ID`, `VITE_OIDC_SCOPE`, `VITE_OIDC_SILENT_REFRESH`,
      `VITE_API_BASE_URL`.
- [x] CLI: `-p/--proxy-auth-config` added to the global options table.
      `PROXYAUTH_DUMP_KUBECONFIG` is only read inside a unit test (not a user
      option), nothing to document.
- [x] `libs/server/api/readme.md` rewritten from the current routes. Was: lists non-existent `/auth/refresh` and
      `/auth/logout`, uses `/api/cluster` instead of `/api/v1/clusters`.
- [x] `work-localy.mdx` referenced `task k3d:up` (doesn't exist); now
      `task k3d:create` / `task k3d:start`.
- [ ] Root `README.md` "Left to do" duplicates `roadmap.mdx` (English vs
      French, not word-for-word): maintainer to pick the single source of truth.
- [x] Documented: Helm values (`helm-chart.mdx`), developer tasks
      (`developper-guide/tasks.mdx`), `/about` (`ui.mdx`). Was: front `/about` page, Helm `values.yaml`
      (Scalar `/api/docs`, `RUST_LOG`/OTel and the probes are now in
      `configuration.mdx`), most developer tasks
      (`test:*`, `envtest:*`, `gen:*`, `audit:*`, `template:*`, `tmux:dev`,
      `talos:*`, `bump`, `cli:*`).
