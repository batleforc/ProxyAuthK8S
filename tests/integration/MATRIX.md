# Matrice §6.P → tests

Une ligne par point numéroté d'un §6.P des RFC 0001 à 0010 (RFC 0009 §6.5).
`task it:matrix` (et donc `task lint`) échoue si un point manque, si une ligne
`suite` cite un test absent, si une ligne `hors suite` n'a ni renvoi ni
raison, ou si une `quarantaine` n'a pas d'issue.

Statuts :

- `suite` : prouvé par la suite d'intégration. Rust :
  `rfcNNNN::pN_sujet` (`tests/integration/tests/rfcNNNN.rs`) ; Playwright :
  `e2e-it/<fichier> › <titre>` (`apps/front/e2e-it`).
- `hors suite` : prouvé ailleurs (job CI, test unitaire) ou à la main, avec
  le renvoi ou la raison.
- `quarantaine` : test instable désactivé, avec son issue (`#N`).
- `à écrire` : accepté seulement pour une RFC dont le code n'est pas encore
  écrit (`PENDING` de `.it/matrix.py`) ; sa PR livre la ligne avec le code.

| RFC  | Point | Statut      | Preuve |
| ---- | ----- | ----------- | ------ |
| 0001 | 1     | hors suite  | job front-lint `check` (`pnpm install --frozen-lockfile`) |
| 0001 | 2     | hors suite  | job front-lint `check` (`task js:ci`) |
| 0001 | 3     | hors suite  | preuve ponctuelle de la migration (comparaison au build NX de `main`) |
| 0001 | 4     | hors suite  | workflow docs-deploy |
| 0001 | 5     | hors suite  | job front-lint `eslint` (rapport SARIF) |
| 0001 | 6     | hors suite  | réglage `allowBuilds` de `pnpm-workspace.yaml`, refus vérifié à la main |
| 0001 | 7     | hors suite  | réglage `minimumReleaseAge` de `pnpm-workspace.yaml`, refus vérifié à la main |
| 0001 | 8     | hors suite  | NX retiré (RFC 0001 PR2) ; `grep` à la main |
| 0001 | 9     | hors suite  | aucun hoisting (`pnpm-workspace.yaml`), refus vérifié à la main |
| 0001 | 10    | hors suite  | job back-dep-audit (`task audit`) |
| 0002 | 1     | suite       | `e2e-it/csp.spec.ts › fonts come from the front only` |
| 0002 | 2     | suite       | `e2e-it/login.spec.ts › full login` |
| 0002 | 3     | hors suite  | `apps/front/e2e/views.spec.ts` (thème avant la première peinture), indépendant du backend |
| 0002 | 4     | hors suite  | `apps/front/e2e/views.spec.ts` (clavier jusqu'à « Copier », annonce), indépendant du backend |
| 0002 | 5     | hors suite  | `apps/front/e2e/views.spec.ts` (360px sur chaque vue) |
| 0002 | 6     | hors suite  | `apps/front/e2e/views.spec.ts` (garde de `/cli`) |
| 0002 | 7     | hors suite  | `apps/front/e2e/views.spec.ts` (axe par vue et par thème) |
| 0002 | 8     | suite       | `rfc0002::p8_the_front_image_serves_its_security_headers` |
| 0002 | 9     | suite       | `e2e-it/csp.spec.ts › an injected inline script is refused` |
| 0003 | 1     | suite       | `rfc0003::p1_port_forward_on_an_allowed_port_connects` |
| 0003 | 2     | suite       | `rfc0003::p2_port_forward_on_another_port_is_refused` |
| 0003 | 3     | suite       | `rfc0003::p3_pod_proxy_on_a_port_is_refused` |
| 0003 | 4     | suite       | `rfc0003::p4_service_proxy_on_a_named_port_is_refused` |
| 0003 | 5     | suite       | `rfc0003::p5_nodes_proxy_is_refused_under_restriction` |
| 0003 | 6     | suite       | `rfc0003::p6_an_explicit_nodes_proxy_rule_lets_it_through` |
| 0003 | 7     | suite       | `rfc0003::p7_without_restriction_everything_passes` |
| 0004 | 1     | suite       | `rfc0004::p1_an_allowed_command_runs` |
| 0004 | 2     | suite       | `rfc0004::p2_a_command_outside_the_list_is_refused` |
| 0004 | 3     | suite       | `rfc0004::p3_a_tty_is_refused` |
| 0004 | 4     | suite       | `rfc0004::p4_an_unnamed_container_is_refused` |
| 0004 | 5     | suite       | `rfc0004::p5_cp_to_the_pod_is_refused` |
| 0004 | 6     | suite       | `rfc0004::p6_a_forbidden_channel_closes_the_session` |
| 0004 | 7     | suite       | `rfc0004::p7_a_repeated_parameter_is_refused` |
| 0004 | 8     | suite       | `rfc0004::p8_an_empty_boolean_is_true` |
| 0004 | 9     | suite       | `rfc0004::p9_rules_never_combine` |
| 0004 | 10    | suite       | `rfc0004::p10_a_broad_rule_grants_no_exec` |
| 0004 | 11    | suite       | `rfc0004::p11_attach_needs_its_own_rule` |
| 0004 | 12    | suite       | `rfc0004::p12_the_kubelet_exec_is_refused` |
| 0004 | 13    | suite       | `rfc0004::p13_without_an_exec_rule_nothing_changes` |
| 0005 | 1     | hors suite  | job repo-checks (includes distants de `Taskfile.yaml`) |
| 0005 | 2     | hors suite  | jobs repo-checks, back-clippy, back-dep-audit et back-cargo-test `coverage` |
| 0005 | 3     | hors suite  | `task docs:crds` dans `task recu` (hook de pré-commit), export du site |
| 0005 | 4     | hors suite  | workflow docs-deploy, site publié vérifié à la main (0005 §17) |
| 0005 | 5     | hors suite  | `checksum:` des includes de `Taskfile.yaml`, refus vérifié à la main |
| 0005 | 6     | hors suite  | `deny.toml` (`sources`), job back-dep-audit |
| 0005 | 7     | hors suite  | `pnpm peers check` dans `js:lint` (`task lint`, job front-lint) |
| 0005 | 8     | hors suite  | `task audit`, refus vérifié à la main |
| 0005 | 9     | hors suite  | `task kit-sync TAG=v0.20.0`, vérifié à la main (0005 §17) |
| 0005 | 10    | hors suite  | `grep` à la main ; npm absent des Taskfiles et workflows |
| 0006 | 1     | suite       | `rfc0006::p1_reads_and_watches_pass` |
| 0006 | 2     | suite       | `rfc0006::p2_delete_is_refused_with_its_verb` |
| 0006 | 3     | suite       | `rfc0006::p3_debug_is_refused` |
| 0006 | 4     | suite       | `rfc0006::p4_run_is_refused` |
| 0006 | 5     | suite       | `rfc0006::p5_exec_passes_on_its_rule` |
| 0006 | 6     | suite       | `rfc0006::p6_a_rule_without_verbs_is_read_only` |
| 0006 | 7     | suite       | `rfc0006::p7_kubelet_exec_through_nodes_proxy_is_refused` |
| 0006 | 8     | suite       | `rfc0006::p8_options_is_refused` |
| 0006 | 9     | suite       | `rfc0006::p9_method_override_does_not_change_the_verb` |
| 0006 | 10    | hors suite  | job back-cargo-test (`tests/proxy_security.rs` et voisins, inchangés) |
| 0007 | 1     | hors suite  | job back-cargo-test (`libs/server/api/tests/http_contract.rs`) |
| 0007 | 2     | hors suite  | `task gen:swagger-file` puis `git diff --exit-code swagger.json`, dans la PR de la migration |
| 0007 | 3     | suite       | `rfc0007::p3_kubectl_streams_work_through_the_tunnel` |
| 0007 | 4     | suite       | `rfc0007::p4_a_port_forward_outside_the_policy_is_refused` |
| 0007 | 5     | suite       | `rfc0007::p5_an_upgrade_with_a_body_is_refused` |
| 0007 | 6     | suite       | `rfc0007::p6_a_spdy_upgrade_is_refused` |
| 0007 | 7     | suite       | `rfc0007::p7_a_foreign_cors_preflight_is_refused` |
| 0007 | 8     | suite       | `rfc0007::p8_the_http_metrics_keep_their_route_label` |
| 0007 | 9     | hors suite  | manuel : `rfc0007::p9_exec_throughput` (ignoré, lancé à la main, 0007 §17), et 7 jours en recette |
| 0008 | 1     | suite       | `rfc0008::p1_rust_log_does_not_silence_the_audit` |
| 0008 | 2     | suite       | `rfc0008::p2_a_watch_is_audited_with_its_ids` |
| 0008 | 3     | suite       | `rfc0008::p3_a_login_is_audited` (auth_login, auth_callback) ; `rfc0008::p3_oc_login_through_the_api_host` (flux OAuth médié piloté par oc login --web, #277) |
| 0008 | 4     | suite       | `rfc0008::p4_an_invalid_code_is_denied` |
| 0008 | 5     | suite       | `rfc0008::p5_a_ban_is_audited` |
| 0008 | 6     | suite       | `rfc0008::p6_a_refused_verb_is_audited` |
| 0008 | 7     | suite       | `rfc0008::p7_the_upstream_refusal_reason_is_kept` |
| 0008 | 8     | suite       | `rfc0008::p8_a_session_end_is_audited` |
| 0008 | 9     | suite       | `rfc0008::p9_the_token_never_reaches_the_output` |
| 0009 | 1     | suite       | `rfc0009::p1_front_answers_through_the_ingress_with_a_csp` |
| 0009 | 2     | suite       | `rfc0009::p2_cli_login_reaches_pods_as_the_dex_user` |
| 0009 | 3     | suite       | `rfc0009::p3_refusals_come_from_the_proxy` |
| 0009 | 4     | hors suite  | revue ; `it:matrix` refuse une dépendance de chemin de `tests/integration` |
| 0009 | 5     | hors suite  | `uv run --script .it/matrix.py --self-test` (dans `task lint`) |
| 0009 | 6     | hors suite  | `uv run --script .it/matrix.py --self-test` (dans `task lint`) |
| 0009 | 7     | suite       | `e2e-it/login.spec.ts › full login` |
| 0009 | 8     | hors suite  | `retries = 0` du profil nextest `it` et `retries: 0` de `playwright.it.config.ts` ; échec volontaire vérifié à la main |
| 0009 | 9     | hors suite  | résumé du job `integration` (durées de `it:up` et `it:test`) |
| 0010 | 1     | à écrire    | `rfc0010::p1_…` |
| 0010 | 2     | à écrire    | `rfc0010::p2_…` |
| 0010 | 3     | à écrire    | `rfc0010::p3_…` |
| 0010 | 4     | à écrire    | `rfc0010::p4_…` |
| 0010 | 5     | à écrire    | `rfc0010::p5_…` |
| 0010 | 6     | à écrire    | `rfc0010::p6_…` |
| 0010 | 7     | à écrire    | `rfc0010::p7_…` |
| 0010 | 8     | à écrire    | `rfc0010::p8_…` |
| 0010 | 9     | à écrire    | `rfc0010::p9_…` |
| 0011 | 1     | à écrire    | `rfc0011::p1_…` |
| 0011 | 2     | à écrire    | `rfc0011::p2_…` |
| 0011 | 3     | à écrire    | `rfc0011::p3_…` |
| 0011 | 4     | à écrire    | `rfc0011::p4_…` |
| 0011 | 5     | à écrire    | `rfc0011::p5_…` |
| 0011 | 6     | à écrire    | `rfc0011::p6_…` |
| 0011 | 7     | à écrire    | `rfc0011::p7_…` |
| 0011 | 8     | à écrire    | `rfc0011::p8_…` |
| 0011 | 9     | à écrire    | job `back-clippy` (`cargo tree -e features -i k8s-openapi`) |
| 0011 | 10    | à écrire    | `task audit` (`cargo deny check sources`) |
