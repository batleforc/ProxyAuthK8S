# RFC 0014 — Annexe d'implémentation

| Field | Value |
| --- | --- |
| RFC | [0014 — Le proxy comme authentificateur de confiance](0014-trusted-authenticator.md) |
| Status | celui de la RFC |
| Rôle | le plan de construction : fichiers, types, chemins, phases |

Ce qui est ici découle des décisions de la RFC ; un écart entre les deux se
corrige dans la RFC d'abord. Les numéros de section (§6.1 à §6.6, §16) sont
ceux que la RFC cite.

---

## 6. Detailed design

### 6.1 CRD (`libs/server/crd/src/`)

- `upstream_auth/mod.rs` (nouveau) : `UpstreamAuth { mode, user_prefix,
  group_prefix, issuer_key }`, `UpstreamAuthMode { Passthrough, FrontProxy,
  Issuer }` (même style que `ValidateAgainst` ; `TokenExchange` et le champ
  `token_exchange` arrivent avec la RFC 0010, décision 47),
  `IssuerKeySettings { issuer_url, token_ttl_seconds, rotation_days }`, avec
  `#[serde(default)]`, les règles CEL et les `maxLength` de §4.3.
- `lib.rs` : `ProxyKubeApiSpec::upstream_auth: Option<UpstreamAuth>` ;
  `ProxyKubeApi::validate` appelle `UpstreamAuth::validate(&self.spec)`, qui
  lit `client_cert`, `auth_config` et `claim_mappings.extra` (règles
  croisées), et le chemin d'`issuer_url` (§4.3) ; colonne `Upstream` en
  `-o wide`.
- `authentication_configuration/oidc_provider.rs` : `claim_mappings:
  Option<ClaimMappings>`.
- `status/mod.rs` : `upstream_auth: Option<UpstreamAuthStatus>`, sérialisé
  `upstreamAuth` en camelCase (§4.5, décision 54), publié par `fields()` et
  envoyé en `null` quand il disparaît (comme `warnings`,
  `status/mod.rs:105`).
- RFC 0010, plus tard : `TokenExchange` sera déclaré dans
  `upstream_auth/token_exchange.rs` au lieu de
  `authentication_configuration/` ; son contenu est celui de la 0010 §6.1.

### 6.2 L'identité (`libs/server/api/src/model/`)

- `User` reçoit `extra: BTreeMap<String, Vec<String>>` et
  `#[derive(Default)]` (`user.rs:136`) ; les six littéraux `User { … }`
  (`forwarded.rs:163,297`, `upgrade.rs:650`, `user.rs:252,312,454`) prennent
  `..Default::default()`.
- `JwtAuthenticators` : `mapped.extra`, dont seul `email` est lu aujourd'hui
  (`user.rs:252-263`), remplit `User.extra`. La clé `email` y sert à
  l'affichage et n'a pas de préfixe de domaine : elle n'est **pas**
  transmise (seules les clés de `claim_mappings.extra` le sont, décision 26).
- Le cache des jetons validés (`token_cache_key`, `oidc_cache.rs:141`, appelé
  à `user.rs:369-381`) ne connaissait ni le cluster ni le mapping : sa clé
  gagne une **empreinte du `claim_mappings`** (SHA-256 de sa forme JSON
  canonique ; absent = une valeur fixe, `legacy`), passée à
  `get_user_info_from_oidc_token` avec le mapping (décision 38). Deux
  clusters au même mapping partagent l'entrée, deux mappings différents
  jamais ; éditer le mapping change la clé, donc agit tout de suite. Le
  dashboard (`state.oidc_client`, `user.rs:160`), sans mapping, garde la
  valeur `legacy`. Livré en phase 1, avec le mapping.
- `OidcProvider` : `/userinfo` est lu en `serde_json::Value` brut quand
  `claim_mappings` est présent : `GroupsUserInfoClaims` ne garde que `groups`
  (`user_claim.rs:5-21`), tout autre claim serait perdu. Les claims passent
  par `map_user`, rendu `pub` dans `jwt_validator` (il est `pub(crate)`).
  Sans mapping, le code actuel.
- `map_user` gagne un paramètre `trusted: bool` : vrai en `FrontProxy` et
  `Issuer`, il exige `email_verified == true` dès que `username` lit
  `email`, par `claim: email` ou par une expression qui contient
  `claims.email` (décision 51). Faux, le comportement d'aujourd'hui, pour les
  deux `validate_against`.
- `UpstreamIdentity::from_user(&User, &UpstreamAuth) -> Result<UpstreamIdentity,
  IdentityRefusal>` (nouveau, `cluster/redirect/upstream_identity.rs`, `pub`
  pour la cible de fuzz) : préfixes, puis refus de §4.2. Pure, sans E/S.

### 6.3 Dans `redirect` (`kube_redirect.rs`)

- Avant la porte 1 : un cluster dont le mode n'est pas dans `allowedModes`,
  que ce réplica ne sait pas servir, ou dont `Ready=False` porte une raison de
  §4.3 qui ferme, répond `503` avec le message de §4.2 (décisions 34 et 45).
- La recherche du cluster (`kube_redirect.rs:69-79`) lit d'abord la clé
  Redis des clusters hors `Passthrough` (décision 42, §6.4), puis la clé
  d'aujourd'hui.
- Une réponse `401` de l'apiserver en `FrontProxy` ou `Issuer` devient le
  `502` de §4.2 (décision 46), sur les chemins standard et virtuel ; sur le
  chemin upgrade, la poignée refusée reçoit le même `502`.
- Le chemin upgrade retire de `Sec-WebSocket-Protocol` les entrées
  `base64url.bearer.authorization.k8s.io.*`, dans tous les modes, en
  gardant les autres valeurs dans leur ordre (décision 39).
- Après la porte 8, en `FrontProxy` et `Issuer` : calcul d'`UpstreamIdentity` ;
  un `IdentityRefusal` donne le `403` de §4.2, sans appel en amont.
- `RedirectContext` (`kube_redirect/context.rs:23`) reçoit
  `upstream: UpstreamCredential` :
  `Passthrough | Bearer(SecretString) | Headers(UpstreamIdentity)`. La
  RFC 0010 y range son jeton échangé en `Bearer` (au lieu du champ
  `upstream_token` de son §6.3), l'`Issuer` aussi.
- Le point de couture est **double**. `identity_headers` (`forwarded.rs:145`)
  ne rend aujourd'hui que les `x-forwarded-*` ; l'`Authorization` du client
  est copiée par la boucle de chaque appelant, `apply_forward_headers`
  (`upstream.rs:141-158`) et `build_upgrade_request`
  (`upgrade/connect.rs:207-211`, `authorization` n'étant pas un en-tête
  filtré, `forwarded.rs:198`). Donc : `identity_headers` reçoit
  `&UpstreamCredential` et rend les en-têtes à poser ; un prédicat partagé,
  à côté de `is_upstream_auth_header`, fait sauter `authorization` aux deux
  boucles hors `Passthrough`. Le filtrage `is_upstream_auth_header` reste
  appliqué avant, dans tous les cas. Changer `identity_headers` seul
  laisserait le jeton IdP partir (T5).
- `UpstreamCredential` atteint tous les chemins : `RedirectContext` pour le
  relais standard et l'upgrade, `Caller` pour les API virtuelles
  (`virtual_redirect/mod.rs:255`) et `ListFallbackArgs` pour les revues de
  `list_fallback` (`virtual_redirect/mod.rs:400-411`, `review.rs:61,198`), qui
  ne passent pas par `RedirectContext`. Les tests unitaires qui figent la
  signature d'aujourd'hui (`forwarded.rs:284,296`, `upgrade.rs:747,824`)
  changent avec elle.
- `list_fallback/cache.rs:21-58` : la clé de cache des résultats de revue
  (`identity_fingerprint`, qui ne hache que nom et groupes) inclut le mode,
  les préfixes et les `extra` (qu'un webhook d'autorisation peut lire), pour
  qu'un changement de mode ne serve pas des résultats calculés sous l'ancienne
  identité.
- `common/src/upstream_cache.rs` : l'entrée d'un client à certificat expire
  au plus tard au `not_after` du certificat (`min(ttl, not_after − now)`, avec
  un plancher de 30 s une fois le certificat expiré, sinon un TTL nul
  désactive le cache et chaque requête relit le Secret). `not_after` n'est
  connu qu'après la construction : `get_or_try_insert_with`
  (`upstream_cache.rs:125`), qui fixe le TTL avant, gagne une variante dont le
  constructeur rend `(valeur, ttl)` ; `upstream_client` réinsère la config TLS
  avec ce même TTL. Un certificat renouvelé dans le même Secret ne sert qu'aux
  **nouvelles** connexions, au plus `PROXY_UPSTREAM_CLIENT_TTL` (300 s,
  `upstream_cache.rs:15-18`) plus tard, car l'empreinte du cache porte sur la
  référence, pas sur le contenu (`upstream_cache.rs:78-82`) ; une connexion
  déjà ouverte garde l'ancien jusqu'à sa fermeture. Documenté (§13).
- `kube_redirect/tls.rs` : `build_tls_config` prend son certificat client,
  en `FrontProxy`, du Secret `proxyauth-frontproxy-<ns>.<nom>` du namespace du proxy
  (label `managed-by` contrôlé, annotation de cible comparée à
  `spec.service`), jamais de `client_cert` (refusé à
  l'admission, décisions 40 et 49) ; `UpstreamCacheKey::for_proxy`
  (`upstream_cache.rs:60-82`) porte cette référence dans son empreinte.

### 6.4 Contrôleur (`libs/server/controller/src/proxy_kube_api/`)

- `FrontProxy` (effectif ou en préparation) : lit le Secret
  `proxyauth-frontproxy-<ns>.<nom>` du namespace du proxy, compare son annotation de
  cible à `spec.service`, parse le certificat
  (`x509-parser` 0.18, nouvelle dépendance directe du contrôleur, déjà dans
  `Cargo.lock` par `libs/agent/pki`) pour `not_after`, CN et O ; pose
  Conditions et statut, dont `clientCertSecret`, le nom à créer (§4.3,
  §4.5). Requeue au plus tard à `not_after` et à J−30.
- `Issuer` / `issuer_key` : crée et fait tourner la clé (§5.3), recopie
  `issuer_url` dans le statut, publie `status.upstreamAuth`, balaie les
  Secrets orphelins.
- Sonde (§5.4) : dans le processus `api`, sur le leader (décision 41) ; le
  réconciliateur, dans le même processus, lit son dernier résultat pour
  `UpstreamTrusted` et `Ready`.
- `allowedModes` : `Ready=False` (`UpstreamModeNotAllowed`) hors liste ;
  `OIDC_AUDIENCE_VALIDATION` différent d'`enforce` en mode effectif :
  `Ready=False` (`AudienceValidationRequired`) (décision 50).
- **Publication Redis** (décision 42) : un cluster dont le mode n'est pas
  `Passthrough` est publié sous une clé nouvelle (préfixe
  `<REDIS_PREFIX>upstream-auth:`, `crd/src/lib.rs:33`), et sa clé
  d'aujourd'hui est supprimée. Un binaire d'avant la phase 1 ne lit que
  l'ancienne : il ne trouve pas le cluster et répond « cluster inconnu »,
  au lieu de relayer le jeton IdP. Retour en `Passthrough` : l'inverse.
- **Démarrage** : le contrôleur lit le schéma installé de la CRD et refuse de
  démarrer s'il ne connaît pas `upstream_auth` (CRD rétrogradée qui élaguerait
  le champ), avec un message qui nomme la version de `chart-crd` attendue.
- RBAC (`deploy/chart/templates/rbac.operator.yaml`) : un **Role** dans le
  namespace du proxy, hors de la condition `oneShotToken.enabled` (les verbes
  `create`/`patch` des lignes 60-61, dans le bloc 51-62, n'existent qu'avec
  elle), avec `get`, `create`, `update`, `delete` et `list` sur `secrets`. Le
  RBAC ne sait pas filtrer par label : `list` lit et `delete` atteint **tous**
  les Secrets du namespace du proxy, pour un ServiceAccount que le proxy
  partage ; accepté, le namespace du proxy est celui de l'admin du proxy
  (T18, décision 43). Le même `get` sert au certificat `FrontProxy`. Lire le
  schéma de la CRD au démarrage demande `get` sur
  `customresourcedefinitions`, limité par `resourceNames` à celle du
  `ProxyKubeApi`. Aucun droit nouveau sur les Secrets des namespaces des
  tenants ; le `get` existant (lignes 63-75) reste `get` seul. Le proxy et le
  contrôleur partagent un ServiceAccount (`dp.back.yaml:23`) : « le proxy
  n'écrit jamais la clé » est tenu par le code, pas par le RBAC (§7.1, T18).

### 6.5 Émetteur (`libs/server/api/src/issuer/`)

- `discovery` et `jwks` : handlers axum sous `/issuer/{ns}/{cluster}`, hors
  utoipa ; répondent depuis le cache en mémoire (§4.4), `Cache-Control:
  max-age=60`. `{ns}/{cluster}` → UID par la recherche de cluster existante
  (Redis, `kube_redirect.rs:70`) ; si Redis ou l'API kube ne répondent pas,
  le magasin « dernier bon document » de §4.4 sert, borné par `retire_at`
  (décision 44).
- `sign(identity, cluster) -> SecretString`, appelé par `redirect` pour
  `UpstreamCredential::Bearer`.

### 6.6 Banc (`.it/`)

- `FrontProxy` : une **CA dédiée** au run ; son certificat est concaténé à la
  `request-header-ca.crt` de k3s dans un fichier passé par
  `--kube-apiserver-arg=requestheader-client-ca-file=…` et
  `requestheader-allowed-names=system:auth-proxy,proxyauth-it` dans
  `.it/k3d.yaml.tpl` (qui ajoute déjà `authentication-config`), pour ne pas
  casser l'agrégation de k3s. `it:up` émet le certificat client
  `CN=proxyauth-it` dans le Secret `proxyauth-frontproxy-<ns>.<nom>` du namespace du
  proxy, avant même le `ProxyKubeApi` de test (le nom ne dépend pas de
  l'UID). Sondé (décision 58) : la concaténation garde l'agrégation.
- `Issuer` : une entrée de plus dans l'`AuthenticationConfiguration` de
  `.it/k3d.yaml.tpl` (avec ses `userValidationRules`), issuer
  `https://proxyauth.k8s.localhost:8443/issuer/…`. Le nœud résout déjà ce nom
  vers Traefik (`hostAliases`, lignes 17-23) et la CA du run est déjà
  injectée (`__CA__`). L'apiserver recharge ce fichier à chaud (Kubernetes
  1.30 et plus). Le banc héberge donc l'issuer dans le cluster qui le croit,
  ce que §10.2 déconseille en production : acceptable pour un banc jetable,
  et c'est le cas qui exerce le démarrage à froid.
- Un **écho amont** (un pod qui renvoie les en-têtes reçus, exposé comme
  `ProxyKubeApi` de test) : c'est par lui que §6.P prouve ce qui part, ou
  ne part pas, vers l'amont (points 3, 16 et 17).
- L'audit de l'apiserver du banc est activé (politique `Metadata`, journal
  lu par `task it:logs`) : c'est par lui que §6.P prouve qu'une requête n'est
  **pas** arrivée en amont, et quel `Authorization` est arrivé.
- Le banc est partagé, et tourne aussi dans Release Prepare : la sonde k3s
  (décision 58) a montré que la concaténation ne casse pas l'agrégation, le
  banc partagé suffit.
- `.it/values.yaml` : `upstreamAuth.allowedModes: [Passthrough, FrontProxy,
  Issuer]`, et `OIDC_AUDIENCE_VALIDATION=enforce`.
- Personas (`.it/personas.yaml`) : un persona dont Dex renvoie le groupe
  `system:masters`, un dont le claim de nom est absent. Les mots de passe
  statiques de Dex ne choisissent pas leurs groupes : le premier passe par le
  connecteur qui porte déjà les groupes des personas ; le second s'obtient
  par un `claim_mappings.username` qui vise un claim que Dex n'émet pas.

---

## 16. Implementation phases

| Phase | Content |
| --- | --- |
| 1 | `claim_mappings` sur `OidcProvider` (`/userinfo` brut et plafonné, `map_user` public avec `trusted` et `email_verified`) et son empreinte dans la clé du cache des jetons (décision 38) ; `User.extra` et `Default`, `extra` des `JwtAuthenticators` ; le champ `upstream_auth` (CRD sans `TokenExchange`, CEL, colonne `-o wide`, statut `upstreamAuth.mode` en camelCase) avec `allowedModes` (décision 47) et la **garde** : `503` pour tout mode non autorisé ou non servi ; clé Redis des clusters hors `Passthrough` et refus d'une CRD sans le champ (décision 42) ; retrait du jeton de `Sec-WebSocket-Protocol` dans tous les modes (décision 39) ; `upstream_mode` dans l'audit ; texte de la RFC 0010 amendé (décision 55). §6.P points 1, 2, 15, 16 et 17 ; T19, T20, T23, T24, T26. Utile seul : aligner le nom vu par le proxy, et rendre sûres les mises à jour suivantes. |
| 2 | `FrontProxy` : `UpstreamIdentity` et ses refus, `UpstreamCredential` dans `RedirectContext`, `Caller` et `ListFallbackArgs`, prédicat partagé sur `authorization`, `append` sur le chemin upgrade, clé de cache de `list_fallback`, retrait de `x-forwarded-*`, `401` amont traduit en `502` (décision 46), métriques ; certificat lu du Secret `proxyauth-frontproxy-<ns>.<nom>` du namespace du proxy, lié à sa cible, `client_cert` refusé dans ce mode (décisions 48, 49, 56) ; `enforce` exigé (décision 50) ; contrôleur : certificat (`x509-parser`), Conditions ; sonde dans `api` sur le leader, avec `Unknown` et son poids sur `Ready` (décisions 41, 45) ; cache TLS borné par `not_after` ; tests T1 à T7, T13, T16, T17, T21, T22, T27, T28, T29 ; fuzz ; banc (CA dédiée, audit de l'apiserver, écho amont, persona `system:masters`) ; doc front-proxy, migration, runbook. §6.P points 3 à 9 et 11 (partie `FrontProxy`). |
| 3 | `Issuer`, indépendant de la RFC 0010 : `client_cert` refusé (décision 40) ; `issuer_key.issuer_url` requis et vérifié (décision 53) ; clé déclarative, rotation et révocation `revoke:<nonce>` (décision 52) ; propriété par labels et namespace, balayage en deux temps (décision 43) ; lecture de clé résiliente (décision 44) ; Role dans le namespace du proxy ; routes `/issuer/*` et leur magasin « dernier bon document » ; signature et `TtlCache` ; `UpstreamCredential::Bearer` ; sonde en mode `Issuer` (dont l'identité non préfixée) ; statut ; métriques ; tests T8 à T12, T14, T15, T18, T25 ; banc (`AuthenticationConfiguration` avec l'issuer du proxy et ses `userValidationRules`, deux clusters) ; doc émetteur. §6.P points 10 à 14. |

`UpstreamCredential::Bearer` n'arrive qu'en phase 3 (ou avec la RFC 0010) :
livré en phase 2 sans constructeur, il ferait échouer le lint sur du code
mort.

La RFC 0010 peut être livrée avant, entre ou après les phases 2 et 3 : elle
range sa config sous `upstream_auth` (décision 16) et son jeton dans
`UpstreamCredential::Bearer`.

Chaque contrôle arrive dans la phase de la surface qu'il protège : la garde
de version, `allowedModes`, la clé Redis nouvelle et l'empreinte du mapping
dans le cache en phase 1, avant tout mode de confiance ; les
refus `system:`, nom vide et valeurs non transmissibles en phase 2 avec le
premier en-tête `X-Remote-*`, avec le certificat dans le namespace du proxy
et `enforce` ; la vérification d'`aud`, la clé par cluster, le refus de
`client_cert` en `Issuer` et la non-adoption en phase 3 avec le premier
JWT.
