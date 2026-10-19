# Revue de la roadmap et du README — 2026-10-17

Sources relues : `README.md`, `.docs/content/docs/roadmap.mdx` (source unique de
la roadmap), `todo.md`, `CHANGELOG.md`, `.github/workflows/*` et les issues
ouvertes (`gh issue list`). Chaque statut a été confronté au code. Les
références sont `fichier:ligne`.

> **Alignée le 2026-10-17 sur les grills des RFC 0002 à 0007** et sur
> [PLAN.md](PLAN.md). Les constats sur le code restent ceux de la revue ; seuls
> les renvois aux RFC, la roadmap proposée (§10) et les questions (§11) ont été
> mis à jour. Les mises à jour sont signalées par « → ».

Légende du verdict : **OK** = conforme ; **Exagéré** = la roadmap promet plus
que le code ; **Sous-estimé** = le code fait plus que ce qu'annonce la roadmap ;
**Périmé** = le statut n'est plus à jour.

---

## 1. Synthèse

1. **La v1.0.0 est en grande partie faite, mais affichée « planifiée » (📋).** Le
   bloc sécurité est entièrement ✅, et Redis Cluster est déjà implémenté
   (`libs/server/common/src/redis_pool.rs:25-28,52-73`, feature `cluster` dans
   `Cargo.toml:38`). Il faut passer la section en 🚧, ou déplacer les éléments
   livrés.
2. **Trois affirmations de la v0.1.0 sur la CI sont exagérées** : l'audit CVE
   Rust et le lint front ne tournent pas « à chaque commit », le lint front ne
   bloque jamais (`continue-on-error: true`), et les contrôles « pré-TAG » (cog
   par contributeur humain, changelog obligatoire) ne sont que des hooks locaux.
3. **Plusieurs fonctionnalités livrées n'apparaissent pas dans la roadmap** :
   virtual APIs et découverte OpenShift, filtrage des ports du port-forward,
   list fallback, serveur d'autorisation OAuth intégré pour la CLI, signature et
   SBOM des images, scan Trivy.
4. **La planification de la v0.2.0 se contredit** : #258 (release 0.2.0 «
   Ouroboros ») a pour bloquants #230 (UI, placée en v2.0.0 dans la roadmap),
   #255 (NX, absent de la roadmap) et la PR #253. La roadmap v0.2.0, elle, parle de
   docs, de Krew et de nettoyage git.
5. **Le README a un lien mort, mélange les langues et ne pointe ni vers
   l'installation, ni vers la doc, ni vers la licence.**
6. **Sept mois de travail ne sont pas publiés** : `CHANGELOG.md` s'arrête à
   v0.1.9 (2026-03-20).

---

## 2. v0.1.0 — Fondations (annoncée ✅)

| Élément | Preuve | Verdict | Note |
| --- | --- | --- | --- |
| CRD + validation | CEL `crd/src/lib.rs:44`, `security/allowed_path_configuration.rs:50`, `security/port_range.rs:21` | OK | Les règles CEL sont aussi listées en v1.0.0 : doublon |
| Auth OIDC UI + filtrage par groupe | `dashboard_group` `crd/src/lib.rs:76,124-136`, `visible_clusters/get_all_visible_cluster.rs` | OK | |
| UI : liste, kubeconfig OIDC et non-OIDC | `KubeconfigBlock.vue`, `nosso/NoSsoKubeconfigCard.vue`, `ClusterNoSSOView.vue` | OK | |
| Client API auto-généré | `libs/front/front-api` (HeyApi) | OK | |
| Contrôleur : réconciliation + leader election | `controller/src/lib.rs:19,30` (LeaseLock), `api/tests/envtest_leader_election.rs` | OK | |
| Backend : Redis HA | `common/src/redis_pool.rs` | OK | Gère aussi le mode cluster (§4) |
| Swagger auto-généré | `apps/swaggergen`, `api/src/api_doc.rs` | OK | |
| CodeQL « à chaque commit » | Pas de workflow : c'est le *default setup* GitHub (rust, js/ts, actions, hebdomadaire) | OK sur le fond | Préciser « CodeQL (default setup GitHub) » |
| Analyse CVE Rust « à chaque commit » | `back-dep-audit.yaml` : seulement quand `Cargo.toml`/`Cargo.lock` changent, plus une exécution nocturne | **Exagéré** | Sa liste `ignore:` (l. 30) ne contient que RUSTSEC-2023-0071, alors que `.cargo/audit.toml` ignore aussi h2. Les deux listes divergent. → RFC 0005 (R11) : `cargo deny` remplace `cargo audit`, et `deny.toml` devient la seule liste d'exceptions |
| Clippy en CI | `back-clippy.yaml:55` : `cargo clippy --all-features` en `continue-on-error: true` | **Non bloquant** | Seul `task lint:rust` (`-D warnings`) bloque, et seulement en local (pre-commit). → RFC 0005 (phase 1) : `back-clippy` devient bloquant et appelle `task rust:lint` (`clippy --workspace --all-targets -D warnings`) |
| Lint front « à chaque commit » | `front-lint.yaml:8-14,49` : main et PR vers main, filtré par chemins, `continue-on-error: true` | **Exagéré** | Il ne bloque jamais. Voir aussi le `yarn add` en CI (RFC 0001 §6.4) |
| TAG : Docker → GHCR, Helm → GHCR, draft release | `release-prepare.yml:5,215,404` | OK, mais **cassé aujourd'hui** | Le build nocturne de main échoue depuis le 13 au 15 octobre (Containerfile back : membres `crd_runtime`/`virtual_api`/`jwt_validator` manquants, corrigé dans la PR #253). L'image front est cassée (Yarn 1 au lieu de 4, voir le diagnostic de la PR #253). Une release taguée aujourd'hui échouerait. **Sous-estimé** par ailleurs : signature, SBOM et pré-releases `-rc` ne sont pas mentionnées |
| Pré-TAG : format vX.X.X, cog par contributeur humain, changelog | `cog.toml` (hooks `cog verify`/`cog check`, `pre_bump_hooks`) | **Exagéré** | Uniquement en local ; rien en CI n'impose cog ni le changelog |

## 3. v0.2.0 — Documentation & CLI (annoncée 🚧)

| Élément | Preuve | Verdict | Note |
| --- | --- | --- | --- |
| 10 guides ✅ | `.docs/content/docs/*` | OK, en partie exagéré | `developper-guide/release.mdx` fait 8 lignes et ne parle ni des rc, ni de Krew, ni de la signature. Il n'y a pas de page dédiée à Eclipse Che : le contenu est réparti dans `work-localy.mdx` et `kubectl-plugin.mdx`. `security.mdx`, `virtual-apis.mdx`, `crds.mdx` et `configuration.mdx` existent mais ne sont pas listées |
| Plugin Krew, client généré ✅ | `apps/kubectl_proxyauth`, `libs/cli/client_api` | OK | |
| Publication sur le Krew Index ⏳ | `.krew/proxyauth.yaml.tpl:5` : PR krew-index manuelle, pas de krew-release-bot | OK | Envisager `rajatjindal/krew-release-bot` |
| Manifeste Krew attaché à la release ✅ | `release-prepare.yml:299-343,363,401` | OK, mais **non éprouvé** | Jamais exécuté sur un vrai tag (dernier tag : v0.1.9). La répétition `v0.1.10-rc.1` du `todo.md` est toujours ouverte |
| Nettoyage de l'historique git ⏳ | 354 commits, dont 51 « wip ». `graphify-out/` était suivi (suppression en cours dans l'index) | OK | |

## 4. v1.0.0 — Production Ready (annoncée 📋)

| Élément | Preuve | Verdict | Note |
| --- | --- | --- | --- |
| Tests approfondis | Environ 800 tests Rust (api 320, cli 173, crd 120, common 47, crd_runtime 33, virtual_api 32, controller 31, jwt_validator 29, trace 8, cli_trace 4, server 2 ; 0 dans crdgen, swaggergen et kubectl_proxyauth). nextest + llvm-cov gate (`back-cargo-test.yaml`), envtest (`back-envtest.yaml:89`). Front : 9 specs | **Sous-estimé côté back** | Le vrai trou est le front (9 specs, pas de test a11y ni visuel). C'est traité par la RFC 0002. → Le gate de couverture back (`coverage:ci`, nextest + Redis, seuil 62 %) est conservé par la RFC 0005 (R4). Les RFC 0003, 0004 et 0006 ajoutent des cibles de fuzz (workspace `fuzz/`, job nightly) |
| Redis Cluster HA | `redis_pool.rs:25-28,52-73` (liste d'URLs ou `REDIS_CLUSTER=true`), feature `cluster`, documenté dans `configuration.mdx` | **Périmé** | Déjà livré : à passer en ✅ |
| Sécurité (9 éléments ✅) | `kube_redirect.rs:174-194,221-249`, `throttle.rs`, `audit.rs`, `kube_redirect/tls.rs:43-85`, `jwt_validator`, `oidc_config_source.rs`, `api/tests/oidc_token_validation.rs` | OK | Il manque `allowed_ports` (port-forward). La RFC 0003 révèle des contournements à afficher en ⏳. → Sécurité à venir : RFC 0003 (ports), 0006 (verbes), 0004 (exec/attach), dans cet ordre (PLAN.md) |
| Suppression de `allowed_ressources` ⏳ | toujours présent `crd/src/security/mod.rs:50-57,78`, `envtest.rs:362` | OK | Changement cassant. → Bascules de la v1.0.0 (PLAN.md) : `allowed_ressources`, `verbs` absent = lecture seule (0006 R4), `exec` absent = interdit (0004, décision 1). La RFC 0003 ne bascule plus en v1.0.0 : les règles sans `allowed_ports` sont restreintes dès sa livraison (R2) |
| Publication auto de la doc sur TAG ⏳ | `docs-deploy.yml:4-5` : publication sur push vers `main` et `feat/init-doc` (chemins `.docs/**`) | OK mais **imprécis** | La doc est **déjà** publiée automatiquement depuis main. `feat/init-doc` est un déclencheur résiduel à retirer. `--frozen-lockfile` (Yarn 1) en l. 42 |

**Au niveau de la section** : passer la v1.0.0 en 🚧, ou déplacer ce qui est
livré dans une v0.2.x et ne garder en v1.0.0 que les tests front, les bascules
de défauts cassantes et la publication de la doc par tag.

## 5. v2.0.0 — Évolutions majeures (vision)

| Élément | Preuve | Verdict |
| --- | --- | --- |
| Redesign UI | Issue #230, RFC 0002 | Non commencé. ~~Conflit avec #258~~ → **Tranché** : 0.2.0 (RFC 0002, décision 8) ; la ligne sort de la v2.0.0 |
| Exchange token (fédération d'IdP) | Aucun code. `cluster/auth/oauth/*` est un serveur d'autorisation OAuth pour la CLI, pas de la fédération | Non commencé |
| Support proxy (Netbird) | Aucun code de proxy sortant ni SOCKS | Non commencé |
| Mode agent | `upgrade/tunnel.rs` est le tunnel des upgrades, pas un agent | Non commencé |
| Device code (RFC 8628) | Aucune occurrence de `device_code` ni `device_authorization` | Non commencé (cohérent avec `todo.md`) |

## 6. Livré mais absent de la roadmap

- Virtual APIs et découverte OpenShift (`libs/server/virtual_api`,
  `virtual-apis.mdx`).
- Port-forward avec filtrage des ports (`allowed_ports`, `port_range.rs`,
  `port_forward.rs`), donc **#265 est livré**.
- List fallback (`kube_redirect/list_fallback`).
- Serveur d'autorisation OAuth intégré pour la CLI (`cluster/auth/oauth/*`),
  avec login TOFU, épinglage et rotation de certificat.
- Supply chain : signature des images, SBOM, Trivy, pré-releases.
- CORS deny-by-default (`feat!:` du 2026-10-10).

## 7. Issues ouvertes et roadmap

| Issue | Dans la roadmap ? | Proposition |
| --- | --- | --- |
| #258 Release 0.2.0 « Ouroboros » | v0.2.0, mais avec d'autres bloquants | Aligner la liste de #258 et la section v0.2.0 |
| #230 Rework UI | v2.0.0 | RFC 0002 → **0.2.0** (décision 8) |
| #255 Drop NX / Update | **Absente** | RFC 0001 ; ajouter une ligne « Outillage » en v0.2.0 |
| #265 Filtrer le port-forward par port | **Absente**, et **déjà livrée** | Fermer, en pointant vers la RFC 0003 (R1) ; créer l'issue « Port filtering bypasses » |
| — Filtrage exec/attach | **Absent** (pas d'issue) | RFC 0004 ; créer l'issue ; release proposée 0.5.0 (PLAN.md) |
| — Règles sensibles au verbe | **Absent** (pas d'issue) | RFC 0006 ; créer l'issue ; release proposée 0.3.0, avec la 0003 |
| — Migration actix → axum | **Absent** (pas d'issue) | RFC 0007 ; créer l'issue ; contrat (phase 1) dès la 0006, migration en 0.4.0 |
| — Adoption de weebo-operator-kit | **Absent** (pas d'issue) | RFC 0005 ; créer l'issue, plus une PR côté kit (peers en plages, `NPM_CLIENT`, `KIT_TAG`) |
| #139 Doc auto du chart Helm (helm-docs) | **Absente** | v0.2.0 (Documentation) |
| #23 Refaire la CI/CD (commits signés, scan du code modifié, Sonar) | **Absente**, partiellement couverte | La découper. « Commits signés en CI » rejoint le point « cog en CI » du §2 |
| #87 RUSTSEC-2023-0071 (rsa) | Risque accepté (`security.mdx`) | Lier depuis la roadmap, ou fermer avec un lien vers le risque accepté |
| #243, #244 RUSTSEC-2026-0258 (h2) | Watch list `todo.md` | **Doublons** : en fermer un |
| #263, #268, #269, #270, #271 (rustls) | **Absentes, non triées** | Probablement corrigées par une montée de version : `cargo update -p rustls` puis `task audit` |
| #267 unsafe-libyaml non maintenu | **Absente** | Identifier qui le tire (`cargo tree -i unsafe-libyaml`) |

Les advisories s'accumulent en partie parce que l'`ignore:` de
`back-dep-audit.yaml:30` et `.cargo/audit.toml` divergent. Il faudrait une seule
source, ce qui rejoint `task audit` imposé par le CLAUDE.md. → Réglé par la RFC
0005 : `deny.toml`, exceptions nommées et datées (R11).

---

## 8. README

| # | Constat | Proposition |
| --- | --- | --- |
| R1 | **Lien mort** : `https://batleforc.github.io/weebo-si/0.projects/reverse-api-kube-oidc-based.html` répond **404** (vérifié le 2026-10-17) | Retrouver la nouvelle URL ou retirer le lien |
| R2 | Mélange de langues : le diagramme mermaid est en français (« Utilisateur Front », « Providers d'identité », « Cluster Kubernetes Fédérateur »), le reste en anglais | Choisir une langue pour le README (anglais recommandé : la doc FR est sur le site) |
| R3 | « N'oublier pas de mettre une brique d'exposition… » : faute (« N'oubliez pas »), et la phrase est vague | « Put an ingress / reverse proxy (TLS termination) in front of ProxyAuthK8S » + lien vers `helm-chart.mdx` |
| R4 | « Cluster Kubernetes Fédérateur » est obscur | « Host cluster (ProxyKubeApi CRDs, Lease) » |
| R5 | La section « Local dev with Keycloak » demande d'activer Keycloak et de désactiver Authelia, mais `compose.yaml:6-8` active **les deux**, en contradiction avec son propre commentaire « Choose one ». Le mode Authelia (par défaut) n'est pas documenté | Corriger `compose.yaml`, documenter Authelia comme défaut, et déplacer les trois sections IdP vers `developper-guide/work-localy.mdx` en ne laissant qu'un lien |
| R6 | Aucune section « fonctionnalités » (plugin kubectl, modèle de sécurité, virtual APIs) | 5 à 6 puces avec un lien vers chaque page de doc |
| R7 | Pas d'installation ni de quickstart, alors que `quick-started.mdx` et `helm-chart.mdx` existent | Section « Install » : `helm install …` + `kubectl krew install …` (quand l'index sera publié) |
| R8 | Pas de lien vers la doc principale (seulement vers la roadmap) | Lien vers `https://batleforc.github.io/ProxyAuthK8S/` en tête |
| R9 | `LICENSE` (GPL-3.0), `CONTRIBUTING.md` et `SECURITY.md` existent mais ne sont pas liés ; pas de badge de licence | Section « Contributing / Security / License » + badge |
| R10 | Le README fait 108 lignes, dont environ 60 consacrées au dev local Keycloak/Dex | Le README présente le projet, la doc l'explique (cf. R5) |
| R11 | Le GIF de démo (`.docs/public/demo-1.gif`) montrera l'ancienne UI après la RFC 0002 | À refaire en phase 6 de la RFC 0002. → Avant le `git mv .docs docs` de la RFC 0005 (PLAN.md, G3) |

## 9. `todo.md` et CHANGELOG

- `todo.md` item 1 (« Commit the 2026-10-10 work ») semble fait : commit
  `4f1d427` du 2026-10-11. À cocher ou retirer.
- `todo.md` item 3 et le backlog front attendent la migration batlehub (RFC
  0002).
- `todo.md` duplique le paragraphe device code de la roadmap : garder la roadmap
  comme source et ne laisser qu'un lien (règle « source unique » déjà appliquée
  au README).
- `CHANGELOG.md` s'arrête à v0.1.9 (2026-03-20). La répétition `v0.1.10-rc.1`
  (todo item 2) débloquerait à la fois la release et la vérification du
  manifeste Krew.

## 10. Proposition de roadmap révisée

Alignée sur [PLAN.md](PLAN.md) ; les numéros de 0.3.0 à 0.5.0 restent une
proposition.

```text
v0.2.0 « Ouroboros »  🚧
  Outillage   : sortie de NX, workspace pnpm (RFC 0001)
  UI          : refonte BatleHub (RFC 0002, décision 8), CSP
  Docs        : helm-docs (#139), release.mdx complété, liste des pages à jour
  Krew        : publication sur l'index (krew-release-bot ?)
  CI          : lint front bloquant ?, rc rehearsal
v0.3.0  (feat!)
  Sécurité    : contournements des ports (RFC 0003), règles par verbe (RFC 0006)
  Robustesse  : workspace fuzz/ + job nightly, compteur proxyauth_policy_decisions_total
  Interne     : contrat HTTP de la migration (RFC 0007 phase 1)
v0.4.0
  Serveur     : migration actix → axum, iso-fonctionnelle (RFC 0007)
v0.5.0  (feat!)
  Sécurité    : filtrage exec/attach, audit de session (RFC 0004)
Hors release
  Outillage   : weebo-operator-kit (RFC 0005) : Taskfiles après 0001 + 0003, docs/ après 0.2.0
v1.0.0  🚧 (une grande partie est livrée)
  ✅ Redis cluster, ✅ bloc sécurité
  Bascules de défauts : allowed_ressources supprimé, verbs absent = lecture seule, exec absent = interdit
  Tests front (a11y, e2e)
  Doc publiée par tag
v2.0.0  🔮
  Exchange token, proxy sortant, mode agent, device code
  Fail-open si allow-list vide → future RFC « default: deny » (0003 R9)
  Sujets (users/groups) sur les règles Path (0003 R14)
```

## 11. Questions pour le grill

1. ~~La v1.0.0 passe-t-elle en 🚧, ou les éléments livrés sont-ils déplacés en
   v0.2.x ?~~ → **🚧**, éléments livrés en ✅ sur place (aucun n'est sorti dans
   une release taguée). [x] Appliqué à `roadmap.mdx`, avec les sections v0.3.0 à
   v0.5.0 du §10.
2. ~~L'UI (RFC 0002) est-elle prévue en v0.2.0 (#258) ou en v2.0.0
   (roadmap) ?~~ → **0.2.0** (RFC 0002, décision 8).
3. ~~Langue du README : anglais seul ?~~ → **Anglais seul**, diagramme traduit,
   lien vers la doc (FR) en tête. [x] Appliqué : README réécrit (R2 à R4, R6 à
   R10) ; le lien mort (R1) est retiré, faute de nouvelle URL connue.
4. ~~Les sections Keycloak/Dex du README vont-elles dans la doc ?~~ → **Oui**.
   [x] Appliqué : section « Identity provider » de
   `developper-guide/work-localy.mdx` (Authelia par défaut, Keycloak, Dex), et
   `compose.yaml` n'active plus qu'Authelia (R5).
5. ~~Le lint front doit-il devenir bloquant tout de suite ?~~ → **Non, avec la
   phase 1 de la RFC 0002** : oxlint + ESLint Vue font alors échouer le job
   `check` ; le rendre bloquant avant corrigerait un lint remplacé deux fois
   (RFC 0001 puis 0002).
6. ~~Faut-il une RFC transverse « défauts fermés en v1.0.0 » ?~~ → **Non** : les
   bascules sont listées dans PLAN.md (§ « v1.0.0 »), chacune décidée et
   annoncée par sa RFC ; la RFC 0003 bascule dès sa livraison. Le fail-open de
   l'allow-list vide reste une future RFC.
7. Que faire des advisories rustls (#263, #268 à #271) : montée de version
   immédiate, hors RFC ? → `deny.toml` (RFC 0005) les rendra visibles comme
   exceptions nommées ; la montée de version reste à faire.
8. ~~Faut-il créer maintenant les issues des RFC 0003 (contournements), 0004,
   0005, 0006 et 0007 (§7) ?~~ → **Oui**, textes rédigés ; [ ] à créer par le
   mainteneur (`gh` non configuré ici), puis reporter les numéros dans les RFC
   et `.rfc/README.md`. [ ] Fermer #265.
