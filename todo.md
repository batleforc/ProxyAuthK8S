# TODO — deferred code-health follow-ups

Larger refactors deferred during the desloppify code-health pass (strict score
88.2/100). Each was intentionally left for dedicated, well-tested work rather
than a rushed change. Roughly ordered by score impact / value.

## Architecture

- [ ] **Split the `crd` schema crate from its runtime I/O.** Move the runtime
      methods on `ProxyKubeApi` (`is_reachable`, `get_client`, `to_kube_client`,
      `to_kubeconfig`, `get_oidc_conf`) and the `impl ObjectRedis for ProxyKubeApi`
      out of `libs/server/crd` into a separate crate (or into `api`/`common`) so
      the pure CRD schema no longer depends on `common`/`reqwest`/`kube` client.
      Today `apps/crdgen` transitively links Redis/kube just to print YAML.
      _Drags: cross-module architecture, high-level elegance._

## Error handling & types

- [ ] **Replace `Result<_, String>` with domain error enums** across the ~30
      `map_err`-to-string sites: `crd/lib.rs` runtime methods, `crd/certificate`,
      `crd/service`, and `libs/server/api/src/model/user.rs`
      (`get_user_info*`/`auth_against_*`). Callers currently collapse Redis,
      proxy-not-found, k8s-client and OIDC failures into one opaque 401.
      _Drags: error consistency, api-surface coherence, mid-level elegance._

- [ ] **Propagate CLI exit codes.** Thread a `Result`/exit status through every
      `CliCtx::handle_*` command handler so `Cli::run_cli` returns
      `ExitCode::FAILURE` on a failed command. Today only `handle_get_token`
      propagates, so failed commands still exit 0.

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
- [ ] `task audit` is wired up (`cargo audit` + `yarn npm audit` + `trivy`) and
      currently reports **5 dependency CVEs** — triage and update.
