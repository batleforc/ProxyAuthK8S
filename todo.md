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

## Dependencies (maintainer decisions)

- [ ] Migrate off the deprecated `serde_yaml` (0.9.34+deprecated) — preserve exact
      CRD YAML output. Evaluated 2026-08-29 — **decision: stay on `serde_yaml`.**
      The migration was implemented and fully verified, then deliberately
      reverted; keeping the notes so this is not re-litigated.
      *What the forks actually offer:* all three live forks emit our CRD
      byte-identically (tested directly, not assumed). But `serde_yaml_ng` 0.10
      still sits on the same unmaintained `unsafe-libyaml` 0.2.11 as
      `serde_yaml`, so it is a pure rename with no real gain; `serde_norway`
      0.9.42 swaps in the maintained `unsafe-libyaml-norway` 0.2.15 but has had
      no release since 2024-12, so it trades a "deprecated" label for a stale
      one; `serde_yaml_bw` 2.5.7 is genuinely active but replaces the parser
      engine (`granit-parser`) and adds regex/base64/num-traits to the graph —
      real behavioural risk for the kubeconfig and CLI-config parsing paths.
      *Why staying is defensible:* the 0.9.34 deprecation is the author
      archiving a feature-complete crate, not a defect. It carries **no RUSTSEC
      advisory**, `cargo audit` is clean on it, and our usage is small and
      stable — `to_string`/`from_str` at six call sites (`apps/crdgen`,
      `libs/cli/cli`, one `api` test dep). A frozen dependency doing a frozen
      job is not the same risk as an unmaintained one doing a moving job.
      *Re-open when:* a RUSTSEC advisory lands against `serde_yaml` or
      `unsafe-libyaml`, or our YAML needs grow beyond plain emit/parse (e.g.
      anchors/aliases). The migration is a ~10-line mechanical rename, so it can
      be redone in minutes — `serde_norway` was the drop-in that verified clean;
      re-run the CRD byte-diff and the emitter differential test before
      committing to any fork.

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
