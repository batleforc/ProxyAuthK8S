# Matrice §6.P → tests

Une ligne par point numéroté d'un §6.P des RFC 0001 à 0009 (RFC 0009 §6.5).
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
| 0002 | 1     | à écrire    | |
| 0002 | 2     | suite       | `e2e-it/login.spec.ts › full login` |
| 0002 | 3     | hors suite  | `apps/front/e2e/views.spec.ts` (thème avant la première peinture), indépendant du backend |
| 0002 | 4     | à écrire    | |
| 0002 | 5     | hors suite  | `apps/front/e2e/views.spec.ts` (360px sur chaque vue) |
| 0002 | 6     | hors suite  | `apps/front/e2e/views.spec.ts` (garde de `/cli`) |
| 0002 | 7     | hors suite  | `apps/front/e2e/views.spec.ts` (axe par vue et par thème) |
| 0002 | 8     | à écrire    | |
| 0002 | 9     | à écrire    | |
| 0003 | 1     | à écrire    | |
| 0003 | 2     | à écrire    | |
| 0003 | 3     | à écrire    | |
| 0003 | 4     | à écrire    | |
| 0003 | 5     | à écrire    | |
| 0003 | 6     | à écrire    | |
| 0003 | 7     | à écrire    | |
| 0004 | 1     | à écrire    | |
| 0004 | 2     | à écrire    | |
| 0004 | 3     | à écrire    | |
| 0004 | 4     | à écrire    | |
| 0004 | 5     | à écrire    | |
| 0004 | 6     | à écrire    | |
| 0004 | 7     | à écrire    | |
| 0004 | 8     | à écrire    | |
| 0004 | 9     | à écrire    | |
| 0004 | 10    | à écrire    | |
| 0004 | 11    | à écrire    | |
| 0004 | 12    | à écrire    | |
| 0004 | 13    | à écrire    | |
| 0005 | 1     | hors suite  | job repo-checks (includes distants de `Taskfile.yaml`) |
| 0005 | 2     | hors suite  | jobs repo-checks, back-clippy, back-dep-audit et back-cargo-test `coverage` |
| 0005 | 3     | hors suite  | 0005 phases 2-5 (`task docs:crds`), pas encore livrées |
| 0005 | 4     | hors suite  | 0005 phases 2-5, workflow docs-deploy |
| 0005 | 5     | hors suite  | `checksum:` des includes de `Taskfile.yaml`, refus vérifié à la main |
| 0005 | 6     | hors suite  | `deny.toml` (`sources`), job back-dep-audit |
| 0005 | 7     | hors suite  | 0005 phases 2-5 (`docs/`), pas encore livrées |
| 0005 | 8     | hors suite  | `task audit`, refus vérifié à la main |
| 0005 | 9     | hors suite  | 0005 phases 2-5 (`task kit:sync`), pas encore livrées |
| 0005 | 10    | hors suite  | `grep` à la main ; npm absent des Taskfiles et workflows |
| 0006 | 1     | à écrire    | |
| 0006 | 2     | à écrire    | |
| 0006 | 3     | à écrire    | |
| 0006 | 4     | à écrire    | |
| 0006 | 5     | à écrire    | |
| 0006 | 6     | à écrire    | |
| 0006 | 7     | à écrire    | |
| 0006 | 8     | à écrire    | |
| 0006 | 9     | à écrire    | |
| 0006 | 10    | hors suite  | job back-cargo-test (`tests/proxy_security.rs` et voisins, inchangés) |
| 0007 | 1     | hors suite  | job back-cargo-test (`libs/server/api/tests/http_contract.rs`) |
| 0007 | 2     | hors suite  | `task gen:swagger-file` puis `git diff --exit-code swagger.json`, dans la PR de la migration |
| 0007 | 3     | à écrire    | |
| 0007 | 4     | à écrire    | |
| 0007 | 5     | à écrire    | |
| 0007 | 6     | à écrire    | |
| 0007 | 7     | à écrire    | |
| 0007 | 8     | à écrire    | |
| 0007 | 9     | hors suite  | manuel : débit `kubectl exec … cat /dev/zero`, et 7 jours en recette |
| 0008 | 1     | à écrire    | |
| 0008 | 2     | à écrire    | |
| 0008 | 3     | à écrire    | |
| 0008 | 4     | à écrire    | |
| 0008 | 5     | à écrire    | |
| 0008 | 6     | à écrire    | |
| 0008 | 7     | à écrire    | |
| 0008 | 8     | à écrire    | |
| 0008 | 9     | à écrire    | |
| 0009 | 1     | suite       | `rfc0009::p1_front_answers_through_the_ingress_with_a_csp` |
| 0009 | 2     | suite       | `rfc0009::p2_cli_login_reaches_pods_as_the_dex_user` |
| 0009 | 3     | suite       | `rfc0009::p3_refusals_come_from_the_proxy` |
| 0009 | 4     | hors suite  | revue ; `it:matrix` refuse une dépendance de chemin de `tests/integration` |
| 0009 | 5     | hors suite  | `uv run --script .it/matrix.py --self-test` (dans `task lint`) |
| 0009 | 6     | hors suite  | `uv run --script .it/matrix.py --self-test` (dans `task lint`) |
| 0009 | 7     | suite       | `e2e-it/login.spec.ts › full login` |
| 0009 | 8     | hors suite  | `retries = 0` du profil nextest `it` et `retries: 0` de `playwright.it.config.ts` ; échec volontaire vérifié à la main |
| 0009 | 9     | hors suite  | résumé du job `integration` (durées de `it:up` et `it:test`) |
