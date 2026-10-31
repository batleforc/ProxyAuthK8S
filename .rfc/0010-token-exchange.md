# RFC 0010 — Échange de jeton : une connexion au proxy pour tous les clusters

| Field       | Value                                                                  |
| ----------- | ---------------------------------------------------------------------- |
| Status      | Accepted — implémentée sur #332, preuve du banc en attente (§17)       |
| Short       | Échange de jeton (RFC 8693)                                            |
| Settles     | Ne plus demander une connexion SSO et un jeton stocké par cluster     |
| Author      | Max Batleforc <maxleriche.60@gmail.com>                                |
| Co-author   | Claude Opus 5.5 (1M context)                                           |
| Created     | 2026-10-18                                                             |
| Supersedes  | — (amendée par [0014](0014-trusted-authenticator.md) : config sous `upstream_auth.token_exchange`, `mode: TokenExchange` au lieu d'`enabled`, jeton dans `UpstreamCredential::Bearer`, deux non-goals de §3 levés ; texte amendé en §4.1, §4.3, §4.4, §6.1, §6.3, §6.4, §13) |
| Touches     | `libs/server/crd/src/upstream_auth/` (RFC 0014), `libs/server/common/src/{oidc_cache.rs,token_exchange.rs}`, `libs/server/api/src/cluster/redirect/{kube_redirect.rs,forwarded.rs,audit.rs}`, `…/kube_redirect/{standard.rs,upgrade/connect.rs,virtual_redirect/,list_fallback/}`, `libs/server/api/src/visible_clusters/`, `libs/server/api/src/server_auth/` (nouveau), `libs/server/api/src/metrics.rs`, `libs/cli/cli/src/login/`, `apps/front/src/`, CRD générée, `swagger.json` et clients générés, `.it/`, `tests/integration/`, `.docs/content/docs/` |
| Risk        | high                                                                   |


> Document de référence (environ 5 700 mots) : §1 à §4 suffisent pour en
> discuter ; §5 à §16 servent à l'implémenter et à le relire.

---

## 1. Summary

Aujourd'hui, un utilisateur se connecte au proxy (le jeton du dashboard, émis
par l'IdP du proxy), puis **une fois par cluster** à l'IdP de ce cluster, et
garde un jeton par cluster : dans le trousseau pour le CLI
(`libs/cli/cli/src/keystore.rs`, une entrée par cluster), dans un kubeconfig
téléchargé pour le dashboard. Cette RFC ajoute un mode d'authentification par
cluster, `token_exchange`, où le client présente **le jeton du proxy** et le
proxy l'échange, côté serveur, contre un jeton de l'IdP du cluster
([RFC 8693](https://www.rfc-editor.org/rfc/rfc8693.html)). Le jeton échangé
ne quitte jamais le proxy : il remplace l'`Authorization` de la requête relayée
et vit dans un cache en mémoire jusqu'à son expiration. L'apiserver continue de
valider un vrai jeton de son IdP : le modèle de confiance du cluster ne change
pas.

### Before / after

```yaml
# aujourd'hui : un jeton par cluster, obtenu par un login SSO dédié
users:
  - name: team-a/prod
    user:
      exec: { command: kubectl-proxyauth, args: [get-token, --cluster, team-a/prod] }
      # → renvoie le jeton de l'IdP de team-a/prod, stocké dans le trousseau

# avec cette RFC, sur un cluster en token_exchange : le jeton du proxy
users:
  - name: team-a/prod
    user:
      exec: { command: kubectl-proxyauth, args: [get-token, --server] }
      # → renvoie le jeton du proxy ; aucun jeton du cluster côté client
```

```yaml
# ProxyKubeApi
spec:
  auth_config:
    validate_against: Kubernetes
    oidc_provider:
      enabled: true
      issuer_url: https://sso.example.com/realms/corp
      client_id: kube-prod            # client confidentiel, échange activé
      config_from: { name: kube-prod-oidc }   # client_secret
  upstream_auth:                      # RFC 0014, décision 16
    mode: TokenExchange               # défaut : Passthrough
    token_exchange:
      audience: kube-prod             # le jeton échangé est pour l'apiserver
      max_cache_seconds: 300          # défaut : 300
```

```mermaid
flowchart LR
    subgraph T["aujourd'hui"]
        C1["kubectl<br/>jeton du cluster"] --> P1["proxy<br/>relaie l'Authorization"] --> A1["apiserver"]
    end
    subgraph W["avec cette RFC (cluster en token_exchange)"]
        C2["kubectl<br/>jeton du proxy"] --> P2["proxy<br/>valide, puis échange"]
        P2 -- "RFC 8693" --> I2["IdP du cluster"]
        P2 -- "Authorization remplacée" --> A2["apiserver"]
    end
```

Le jeton que reçoit l'apiserver est, dans les deux cas, émis par l'IdP du
cluster ; ce qui change est qui le détient : le client aujourd'hui, le seul
proxy demain.

---

## 2. Motivation

1. **Une connexion SSO par cluster.** `handle_login_clusters`
   (`libs/cli/cli/src/login/mod.rs:81`) ouvre un navigateur par cluster ; un
   utilisateur de dix clusters fait dix allers-retours chez l'IdP le matin,
   et autant quand les jetons expirent.
2. **Un jeton longue durée par cluster sur le poste.** Le trousseau garde un
   jeton par cluster (`keystore::entry(service, user)`), le dashboard fait
   télécharger un kubeconfig qui en contient un. Chaque jeton est un secret de
   plus qui peut fuir d'un poste, d'une sauvegarde ou d'un partage d'écran.
3. **Deux chemins d'authentification à maintenir côté client.** Le front a
   deux générateurs de kubeconfig (`generateCallbackKubeconfig`,
   `generateCallbackPluginKubeconfig`, `apps/front/src/utils/`) et le CLI trois
   sources de jeton (donné, SSO, saisi). Un cluster en échange n'a besoin que
   du jeton du proxy, que le client a déjà.
4. **Le cas le plus courant s'y prête.** Une entreprise a souvent un seul IdP
   (un realm Keycloak) pour le dashboard et ses clusters ; Keycloak sait
   échanger un jeton du realm contre un jeton pour un autre client du même
   realm depuis la 26.2
   ([notes de version](https://www.keycloak.org/docs/26.2.5/release_notes/)).

---

## 3. Goals / non-goals

**Goals**

- Un utilisateur connecté au proxy atteint tout cluster en `token_exchange`
  sans autre connexion ni autre jeton côté client.
- L'apiserver valide toujours un jeton de son propre IdP ; aucune
  configuration de l'apiserver ne change.
- Le jeton échangé ne quitte jamais le proxy (ni réponse, ni log, ni Redis).
- Opt-in par cluster ; un cluster qui ne l'active pas se comporte exactement
  comme aujourd'hui.
- Chaque échange laisse une trace d'audit et une métrique.
- Le CLI garde **un seul** secret : le refresh token de sa session au proxy,
  rafraîchi sans intervention, obtenu par une connexion SSO au serveur.

**Non-goals**

- **Le proxy comme émetteur de jetons** (option B de l'analyse du
  2026-10-18) : la clé de signature deviendrait la plus sensible du système,
  et chaque apiserver devrait lui faire confiance. Rejeté.
- **L'impersonation** (`Impersonate-*`, `X-Remote-*`, option C) : le proxy
  détiendrait un droit d'impersonation sur chaque cluster. Rejeté ; les
  en-têtes restent filtrés (`is_upstream_auth_header`).
- **La délégation** (`actor_token`, RFC 8693 §4.1) : Keycloak ne l'a qu'en
  preview, et l'apiserver n'en lit rien.
- **Rafraîchir le jeton du dashboard** : le front a déjà `automaticSilentRenew`
  (`apps/front/src/oidc/config.ts`).
- **`offline_access` par défaut** : le refresh token suit la session SSO de
  l'IdP ; l'opérateur qui veut des sessions longues l'ajoute à `OIDC_SCOPES`
  (décision 13).
- **Un mode mixte** où un cluster en échange accepterait aussi un jeton du
  cluster : une seule façon de valider par cluster (décision 1).
- **L'échange pour le dashboard lui-même** (`/api/v1/clusters`) : il utilise
  déjà le jeton du proxy.

---

## 4. User-facing design

### 4.1 Configuration

> **Amendé par la RFC 0014 (décision 55, 2026-10-29).** Le bloc vit sous
> `spec.upstream_auth`, et c'est `upstream_auth.mode: TokenExchange` qui active l'échange : le champ
> `enabled` disparaît, le mode en tient lieu (RFC 0014 §4.1, décisions 16 et
> 47). `TokenExchange` entre dans l'énumération des modes avec cette RFC, et
> doit figurer dans `upstreamAuth.allowedModes` du chart.

Un bloc `token_exchange` sous `upstream_auth`, requis en
`mode: TokenExchange` et refusé dans les autres modes :

```yaml
upstream_auth:
  mode: TokenExchange       # active l'échange ; défaut : Passthrough
  token_exchange:
    audience: ""            # paramètre `audience` envoyé ; vide = omis
    scope: ""               # paramètre `scope` envoyé ; vide = omis
    connector_id: ""        # Dex seulement (§6.2) ; vide = omis
    max_cache_seconds: 300  # plafond de vie d'un jeton échangé en cache
```

- **Mode autre que `TokenExchange`** (dont `upstream_auth` absent) : rien ne
  change, le jeton du client est validé selon `validate_against` et relayé.
- **`mode: TokenExchange`** : le jeton du client est validé contre l'IdP **du
  proxy** (`state.oidc_client`, le même chemin que le dashboard) et
  `validate_against` ne s'applique plus au jeton entrant ; il reste utilisé
  par le contrôleur pour son statut.
- Le client qui échange est celui de `oidc_provider` (`client_id`, et
  `client_secret` ou `config_from`) : un client confidentiel chez l'IdP du
  cluster. Aucun nouveau secret.
- `max_cache_seconds` borne le temps pendant lequel un changement de groupes
  chez l'IdP peut ne pas être vu par l'apiserver (§7, T9). `0` désactive le
  cache : un échange par requête.
- Défaut sûr : `mode: Passthrough`, et `TokenExchange` hors des
  `allowedModes` par défaut du chart. Activer l'échange envoie le jeton du
  proxy de l'utilisateur à l'IdP du cluster ; c'est un choix de l'opérateur.

### 4.2 Behaviour rules

- L'échange a lieu **après** toutes les portes de `redirect` (bannissement,
  authentification, rate limit, groupe, allow-list, ports), juste avant le
  relais. Une requête refusée ne coûte aucun appel à l'IdP.
- Le cache est cherché d'abord ; un jeton échangé n'est réutilisé que pour la
  même paire (jeton du proxy, cluster), et jamais au-delà du plus petit de :
  son `exp` − 30 s, l'`exp` du jeton du proxy, `max_cache_seconds`.
- La requête relayée perd **toutes** les valeurs `Authorization` du client et
  reçoit `Authorization: Bearer <jeton échangé>`. Le reste du filtrage des
  en-têtes (`forwarded.rs`) est inchangé.
- Les trois relais (standard, upgrade, API virtuelle) et les appels faits pour
  l'appelant (`list_fallback`, `SelfSubjectAccessReview`) utilisent le jeton
  échangé.
- Un cluster en échange sans `oidc_provider.enabled` est refusé par
  l'admission (§4.3).

### 4.3 Validation

Rejeté par l'admission (règle CEL, comme `AuthenticationConfiguration`
aujourd'hui) :

| Condition | Rationale |
| --- | --- |
| `mode: TokenExchange` sans `token_exchange`, ou `token_exchange` hors de ce mode | une seule source de vérité pour l'`Authorization` (RFC 0014, décision 16) |
| `mode: TokenExchange` et `oidc_provider.enabled: false` | aucun client pour échanger : chaque requête échouerait |
| `mode: TokenExchange` et `disable_validation: true` | l'échange exige un utilisateur validé ; sans validation, n'importe quel porteur obtiendrait un jeton |
| `max_cache_seconds` > 3600 | borne T9 ; une heure couvre tout IdP raisonnable |
| `audience`, `scope` ou `connector_id` avec CR, LF ou `&` | ces valeurs sont envoyées dans un formulaire ; refusées plutôt qu'échappées |

Avertissements, dans `status.warnings` (vu par `kubectl get proxykubeapi` et
le dashboard) :

| Condition | Comportement | Visible dans |
| --- | --- | --- |
| l'issuer du cluster diffère de celui du proxy et `connector_id` est vide | l'échange part quand même (fédération Keycloak possible) | `status.warnings` |
| `client_secret` absent (ni inline ni `config_from`) | l'échange part en client public, que Keycloak refuse | `status.warnings` |

### 4.4 API surface

| Method | Path | Auth | Who may call | Limit |
| --- | --- | --- | --- | --- |
| GET | `/api/v1/clusters` (champ ajouté `token_exchange: bool` par cluster, **calculé** depuis `upstream_auth.mode == TokenExchange`, RFC 0014 décision 55) | jeton du proxy | inchangé | inchangé |
| GET | `/api/v1/auth/login` | aucune (démarre un SSO) | tout appelant ; `redirect_uri` loopback seulement (comme `/oauth/authorize`) | throttling non scopé |
| GET | `/api/v1/auth/callback` | état à usage unique (Redis, 300 s) | l'IdP du proxy, au retour | throttling non scopé |
| POST | `/api/v1/auth/refresh` | refresh token dans le corps (formulaire) | le détenteur d'un refresh token | throttling non scopé et fail2login (un `invalid_grant` compte) |

Le champ dit au CLI et au front qu'aucune connexion au cluster n'est à faire.
Les trois routes `/api/v1/auth/*` sont la connexion SSO **au serveur** (§6.5).
`swagger.json`, le client HeyApi (`libs/front/front-api`) et `client_api`
sont régénérés.

---

## 5. Architecture

### 5.1 Où se place l'échange

```mermaid
sequenceDiagram
    participant K as kubectl
    participant P as proxy
    participant I as IdP du cluster
    participant A as apiserver
    K->>P: GET /clusters/ns/c/api/v1/pods<br/>Authorization: jeton du proxy
    P->>P: portes 1 à 8 (jeton validé contre l'IdP du proxy)
    alt jeton échangé en cache
        P->>P: cache hit
    else
        P->>I: POST token_endpoint<br/>grant_type=token-exchange, subject_token
        I-->>P: access_token (aud = audience)
    end
    P->>A: GET /api/v1/pods<br/>Authorization: jeton échangé
    A-->>P: 200
    P-->>K: 200 (sans jeton échangé)
```

L'échange arrive après la dernière porte : une requête que le proxy refuse ne
déclenche aucun appel à l'IdP, et le jeton échangé n'existe que pour une
requête que la politique a déjà acceptée.

### 5.2 Le cache

`common::oidc_cache::TtlCache`, déjà utilisé pour les jetons validés : en
mémoire, par processus, borné (`MAX_CACHED_ENTRIES`), purgé à l'expiration.
Clé : SHA-256 du jeton du proxy, de l'UID du `ProxyKubeApi`, de l'issuer, du
`client_id` et de l'`audience`. Le jeton du proxy n'est jamais gardé en clair ;
le jeton échangé l'est, en mémoire seulement.

**Invariant** : parce que la clé contient le hash du jeton présenté, un
utilisateur ne peut obtenir que le jeton échangé de son propre jeton ; parce
que le cache est en mémoire, aucun jeton du cluster n'est écrit dans Redis,
un dump ou une sauvegarde.

Chaque réplica échange pour son compte : avec trois réplicas, au plus trois
échanges par (utilisateur, cluster) et par durée de cache. Les échanges
concurrents sur une même clé sont regroupés derrière un verrou par clé, comme
`list_fallback/cache.rs` le fait déjà.

---

## 6. Detailed design

### 6.1 CRD (`libs/server/crd/src/upstream_auth/`)

> **Amendé par la RFC 0014 (décision 55, 2026-10-29).** Le type vit dans
> `upstream_auth/` et non plus dans `authentication_configuration/` ; `enabled` disparaît (RFC 0014 §6.1).

- `upstream_auth/token_exchange.rs` : `TokenExchange { audience, scope,
  connector_id, max_cache_seconds }`, `#[serde(default)]` ;
  `UpstreamAuthMode` gagne la variante `TokenExchange` et `UpstreamAuth` le
  champ `token_exchange` ; ses règles CEL (§4.3) sur `ProxyKubeApiSpec`.
- `warnings()` étendu pour les deux avertissements de §4.3.

### 6.2 L'appel d'échange (`libs/server/common/src/token_exchange.rs`)

- Un `POST` form-urlencoded vers le `token_endpoint` de la découverte OIDC du
  cluster (déjà en cache, `oidc_config_cache.rs`) : `grant_type=
  urn:ietf:params:oauth:grant-type:token-exchange`, `subject_token`,
  `subject_token_type=urn:ietf:params:oauth:token-type:access_token`,
  `requested_token_type=…:access_token`, et `audience`, `scope`,
  `connector_id` quand ils sont renseignés. Authentification du client en
  `client_secret_basic`.
- Construit avec `reqwest` et `form_urlencoded` (déjà dans l'arbre) : pas de
  nouvelle dépendance ; `openidconnect` 4 n'expose pas la RFC 8693.
- Réponse lue avec une limite (16 Kio), `access_token` et `expires_in`
  extraits ; tout autre champ ignoré. Délai : 5 s.
- Dex : son échange n'accepte qu'un jeton d'un IdP amont, désigné par
  `connector_id`, et seulement par le connecteur OIDC
  ([doc Dex](https://dexidp.io/docs/guides/token-exchange/)). Le cas couvert
  est donc la fédération : l'IdP du proxy est l'amont du Dex du cluster.

### 6.3 Dans `redirect` (`libs/server/api/src/cluster/redirect/kube_redirect.rs`)

> **Amendé par la RFC 0014 (décision 55, 2026-10-29).** Le jeton échangé est
> rangé dans `UpstreamCredential::Bearer` (RFC 0014 §6.3), pas dans un champ
> `upstream_token` ; le point de couture d'`Authorization` est celui de la
> 0014.

- Porte 4 : si `upstream_auth.mode` vaut `TokenExchange`, le jeton est
  validé par
  `User::get_user_info_from_oidc_token(token, data.oidc_client)` au lieu de
  `get_user_info_with_proxy`. Le throttling et `is_caller_fault()` sont
  inchangés.
- Après la porte 8 : `exchanged = token_exchange::exchange(&data, &proxy,
  token).await`, rangé dans `RedirectContext` (`upstream:
  UpstreamCredential::Bearer(SecretString)`).
- `apply_forward_headers` (`upstream.rs`) et `build_upgrade_request`
  (`upgrade/connect.rs`) remplacent `Authorization` pour une
  `UpstreamCredential::Bearer` ; `list_fallback` et `virtual_redirect` la
  reçoivent par `ListFallbackArgs` et `Caller` (RFC 0014 §6.3).

### 6.4 Les clients

- `visible_clusters` : champ `token_exchange` dans la réponse, vrai quand
  `upstream_auth.mode` vaut `TokenExchange`.
- CLI : `login clusters` ne lance pas de SSO pour un cluster en échange et
  écrit un contexte dont l'exec renvoie le jeton du serveur ; `get-token`
  n'a pas de jeton de cluster à chercher et rafraîchit le jeton du serveur
  quand il lui reste moins de 60 s (§6.5).
- CLI : `login --server-url` sans `--token` lance la connexion SSO au serveur ;
  avec `--token`, le jeton collé reste accepté, sans rafraîchissement (scripts,
  CI).
- Front : le bouton de connexion d'un cluster en échange télécharge
  directement le kubeconfig à plugin (`generateCallbackPluginKubeconfig`),
  sans aller-retour chez l'IdP du cluster.

### 6.5 La connexion SSO au serveur (`libs/server/api/src/server_auth/`)

Sans elle, le CLI n'a que le jeton collé au `login --server-url --token` :
avec un Keycloak par défaut, il expire en 5 minutes, et l'échange ne tiendrait
pas sa promesse (décision 8). Elle est **médiée par le proxy** (décision 10),
comme `login clusters` l'est déjà pour un cluster :

1. `GET /api/v1/auth/login?redirect_uri=http://127.0.0.1:<port>/…` : le proxy
   range état, nonce et vérificateur PKCE dans Redis (300 s, usage unique) et
   redirige vers son IdP avec **son** client (`OIDC_CLIENT_ID`, et
   `OIDC_CLIENT_SECRET` s'il en a un). Le jeton obtenu porte donc
   `OIDC_AUDIENCE`, comme celui du dashboard : aucune configuration nouvelle
   chez l'IdP.
2. `GET /api/v1/auth/callback` : le proxy échange le code côté serveur, vérifie
   le jeton d'identité, puis rend jeton d'accès et refresh token au listener
   loopback du CLI.
3. `POST /api/v1/auth/refresh` (`refresh_token` dans le formulaire) : le proxy
   appelle l'IdP avec son client et rend la nouvelle paire.

Le CLI garde la paire dans le trousseau, en une seule entrée par serveur
(`keystore::entry`). Évènements d'audit, au schéma de la RFC 0008 :
`server_login` (allowed, denied, error) et `token_refreshed` ; jamais un jeton.

### 6.P Proof against a real client

Suite d'intégration de la RFC 0009 (`tests/integration/tests/rfc0010.rs`),
le kubectl épinglé du banc (`mise.toml`) et le CLI du dépôt, contre deux IdP
réels : le Dex existant (Dex A, IdP du proxy) et un second Dex (Dex B, IdP du
cluster, même image), dont le connecteur OIDC pointe vers Dex A et qui sert
l'échange (Dex v2.46.0, `server/grants/tokenexchange.go`). L'apiserver de k3s
fait confiance à Dex B par une `AuthenticationConfiguration` ; le RBAC du test
porte sur les groupes, Dex B réencodant le `sub` (décision 11).

1. `kubectl-proxyauth login --server-url …` sans `--token`, puis `kubectl get
   pods` sur un cluster en échange — succès ; le log de Dex B montre une
   requête `token-exchange` ; le trousseau du persona contient une seule
   entrée (la session au serveur) et aucun jeton du cluster.
2. Dix `kubectl get pods` de suite — un seul échange côté Dex B (le cache) ;
   `proxyauth_token_exchange_total{result="cache_hit"}` monte de 9.
3. **Refus** : un persona hors de `proxy_group` — `403` du proxy, et **aucune**
   requête `token-exchange` dans le log de Dex B.
4. **Refus** : un jeton de Dex B (login direct) présenté au proxy pour un
   cluster en échange — `401` ; l'apiserver n'a reçu aucune requête de ce
   persona.
5. `kubectl exec` et `kubectl port-forward` sur un cluster en échange — le
   tunnel s'ouvre ; l'audit de la session porte l'utilisateur du jeton du
   proxy.
6. Dex B arrêté — `kubectl get pods` reçoit `503` (pas `401`), et le compteur
   de fail2login du persona n'a pas bougé.
7. La réponse relayée et les logs du proxy ne contiennent jamais la valeur du
   jeton échangé (recherche de la chaîne dans `task it:logs`).
8. Le jeton d'accès de la session au serveur expiré (durée courte configurée
   sur Dex A pour le test) — `kubectl get pods` réussit quand même : `get-token`
   a appelé `/api/v1/auth/refresh` (ligne `token_refreshed` allowed), sans
   navigateur.
9. **Refus** : `/api/v1/auth/refresh` avec un refresh token invalide — `400`,
   ligne `token_refreshed` denied, et le compteur de fail2login de l'adresse
   monte.

Le cas Keycloak (même realm, internal-to-internal) est vérifié à la main, hors
suite, en suivant la doc ; aucune image Keycloak n'entre sur le banc
(décision 11).

**Deliberately untouched**, so reviewers do not go looking:

- `cluster/auth/oauth/*` — le serveur OAuth médié reste le moyen d'obtenir un
  jeton du cluster pour un cluster *sans* échange.
- `jwt_validator` — l'échange ne valide pas le jeton échangé : l'apiserver le
  fait, c'est le but.
- `forwarded.rs::UPSTREAM_AUTH_PREFIXES` — `Impersonate-*` et `X-Remote-*`
  restent jetés ; cette RFC n'ouvre pas l'impersonation.

---

## 7. Security

**Rating:** high — le proxy obtient, pour chaque utilisateur, un jeton
accepté par l'apiserver du cluster, et l'envoie à sa place.

### 7.1 Threat model

Actifs : le jeton du proxy de l'utilisateur, le jeton échangé, le secret du
client d'échange, l'accès au cluster. Acteurs : un client authentifié, un
client non autorisé sur ce cluster, un tiers qui lit les logs, l'IdP du
cluster.

```mermaid
flowchart LR
    C["client<br/>(non fiable)"] -- "B1 : jeton du proxy" --> P["proxy"]
    P -- "B2 : subject_token + secret client" --> I["IdP du cluster"]
    P -- "B3 : jeton échangé" --> A["apiserver"]
    P -. "jamais" .-> R["Redis, logs, réponses"]
```

| # | Threat (STRIDE) | Where | Mitigation | Residual risk |
| --- | --- | --- | --- | --- |
| T1 | Spoofing : un jeton du cluster ou forgé présenté sur un cluster en échange | B1 | validé contre l'IdP du proxy seul (`get_user_info_from_oidc_token`, porte 4) ; sinon `401` | aucun |
| T2 | Elevation : un non-membre de `proxy_group` obtient un jeton échangé | B1→B2 | l'échange suit la porte 6 (`is_proxy_allowed`) et l'allow-list ; refus avant tout appel | aucun |
| T3 | Info disclosure : le jeton échangé revient au client | B1 | jamais écrit dans la réponse ; l'apiserver ne renvoie pas l'`Authorization` ; `SecretString` sans `Debug` | aucun |
| T4 | Info disclosure : un utilisateur reçoit le jeton échangé d'un autre | cache | clé = hash du jeton présenté + UID du cluster + audience | aucun |
| T5 | Tampering : le client double ou glisse un `Authorization` qui passe au relais | B3 | toutes les valeurs `Authorization` retirées avant d'insérer la nôtre | aucun |
| T6 | DoS : un flot de jetons valides distincts force un échange chacun | B2 | rate limit par utilisateur (porte 5) avant l'échange ; cache borné ; verrou par clé ; délai de 5 s | un utilisateur légitime peut coûter un échange par requête s'il change de jeton à chaque fois ; accepté (opérateur) |
| T7 | Repudiation : un accès par jeton échangé sans trace | P | événement d'audit `token_exchanged` (utilisateur, cluster, résultat, pas de jeton) | aucun |
| T8 | Info disclosure : le jeton du proxy part chez un IdP tiers | B2 | opt-in par cluster ; avertissement si l'issuer diffère sans `connector_id` (§4.3) | l'opérateur qui active l'échange vers un IdP tiers lui confie le jeton ; accepté (opérateur) |
| T9 | Elevation : groupes retirés chez l'IdP, jeton échangé encore en cache | cache | `max_cache_seconds` (300, ≤ 3600) ; l'`exp` du jeton du proxy borne aussi | jusqu'à `max_cache_seconds` ; accepté (opérateur, réglable) |
| T10 | Elevation : un jeton échangé sert hors du proxy | B3 | il ne sort pas du proxy (T3) ; son `aud` est celle du cluster | s'il fuyait, valide jusqu'à son `exp` ; comme un jeton de cluster aujourd'hui |
| T11 | Elevation : l'IdP du cluster échange un jeton de l'IdP du proxy émis à un autre client | B2 | Dex ne vérifie pas l'audience du `subject_token` (`connector/oidc/oidc.go`, `SkipClientIDCheck`) : le proxy le fait avant, à la porte 4 (`OIDC_AUDIENCE`, `token_audience.rs`), et n'envoie à l'échange qu'un jeton validé | qui joint l'IdP du cluster directement avec un jeton de l'IdP du proxy obtient un jeton du cluster, sans le proxy : propre à Dex, documenté ; accepté (opérateur) |
| T12 | Spoofing / DoS : abus de `/api/v1/auth/refresh` (refresh tokens volés ou devinés, inondation) | B1 | throttling non scopé et fail2login par adresse (un `invalid_grant` compte, un IdP injoignable non) ; refresh token dans le corps, jamais dans l'URL ni un log ; l'IdP reste seul juge de sa validité | un refresh token volé sert jusqu'à la fin de la session SSO de l'IdP ; accepté (comme tout refresh token) |

### 7.2 Authentication and authorisation

- L'échange n'existe que sur le chemin de proxy, après authentification et
  autorisation.
- Deux surfaces nouvelles sont joignables sans jeton du proxy :
  `/api/v1/auth/login` (démarre un SSO, `redirect_uri` loopback seulement, le
  même garde-fou que `/oauth/authorize` contre la redirection ouverte) et
  `/api/v1/auth/refresh` (un refresh token est la preuve ; l'IdP en juge). Les
  deux passent par le throttling non scopé avant tout appel à l'IdP (T12).
- Défaut : `mode: Passthrough`, et `TokenExchange` hors des `allowedModes`
  par défaut du chart ; un cluster sans `upstream_auth` n'échange rien.
- Un contournement de la porte 4 donnerait un jeton du cluster à son auteur :
  c'est pourquoi l'échange n'accepte que des requêtes dont `user` est
  `Some` (affirmé dans `exchange`, et `disable_validation` interdit avec
  l'échange, §4.3).

### 7.3 Input handling and injection

- `audience`, `scope`, `connector_id` viennent de la CRD (opérateur) et
  passent par `form_urlencoded` ; CR/LF/`&` refusés à l'admission en plus.
- La réponse de l'IdP est lue avec une limite de 16 Kio et un délai de 5 s ;
  un JSON invalide ou sans `access_token` est un échec (§10.2).
- `token_endpoint` vient de la découverte de l'issuer configuré : pas de
  nouvelle URL contrôlée par un utilisateur, donc pas de nouveau SSRF.

### 7.4 Secrets and cryptography

- Secret du client : le `client_secret` existant, de préférence via
  `config_from` (Secret Kubernetes) ; rotation inchangée.
- Le jeton échangé est en mémoire dans un `SecretString` (crate `secrecy`
  0.10.3, déjà dans `Cargo.lock` en transitif ; ajoutée en dépendance directe
  de `common`, §8), jamais dans Redis, un log, une métrique ou une réponse.
- Le jeton du proxy n'est gardé que haché (SHA-256) comme clé.

### 7.5 Abuse and resource exhaustion

- Un appel sortant au plus par (jeton, cluster, réplica) et par durée de cache.
- Les refus (portes 1 à 8) précèdent l'échange : un appelant non autorisé ne
  fait jamais appeler l'IdP.
- Cache borné en entrées ; une entrée fait environ 2 Kio (un jeton).

### 7.6 Existing defences that still hold

- Le filtrage d'en-têtes de `forwarded.rs` (identité, impersonation).
- Le throttling et `is_caller_fault()` ; l'allow-list et les ports.
- Le mTLS vers l'apiserver (`tls.rs`) : inchangé, le jeton échangé circule
  dans la même connexion.

---

## 8. Supply chain

| Dependency / image / tool | Version | Why this one | Licence | Known CVEs (`task audit`) | Maintenance |
| --- | --- | --- | --- | --- | --- |
| `secrecy` (dépendance directe de `common`) | 0.10.3, déjà dans `Cargo.lock` | un type de secret sans `Debug` ni `Display`, plutôt qu'un type maison | MIT OR Apache-2.0 | aucune (`task audit` vert aujourd'hui) | projet RustCrypto/iqlusion |
| `reqwest`, `form_urlencoded`, `sha2`, `serde_json` | déjà dans l'arbre | l'appel d'échange (`openidconnect` 4 et `oauth2` 5 n'implémentent pas la RFC 8693) | inchangé | inchangé | inchangé |
| second Dex sur le banc | même image que `.it/images.lock` (`dex`) | IdP du cluster pour §6.P, sans nouvelle image | Apache-2.0 | inchangé | inchangé |

- **Build and release** : inchangés ; le banc gagne un déploiement, pas une
  image.
- **Updates** : rien de nouveau à suivre.

---

## 9. Data and privacy

| Data | Personal? | Where stored | Retention | Who can read it | Leaves the instance? |
| --- | --- | --- | --- | --- | --- |
| jeton échangé | oui (identifie l'utilisateur) | mémoire du pod | ≤ `max_cache_seconds` | le processus du proxy | vers l'apiserver du cluster seulement |
| hash du jeton du proxy | non (SHA-256, non réversible) | mémoire du pod | idem | le processus | non |
| jeton du proxy (`subject_token`) | oui | jamais stocké | — | — | **oui : vers l'IdP du cluster** (T8) |
| événement `token_exchanged` | oui (nom d'utilisateur, groupes) | logs (cible `audit`) | celle des logs | l'opérateur | selon la collecte des logs |
| refresh token de la session au serveur | oui (rattaché à l'utilisateur) | trousseau du poste (une entrée par serveur) ; transite par le proxy à chaque rafraîchissement, jamais stocké côté serveur | la session SSO de l'IdP (sans `offline_access`) | l'utilisateur ; le proxy le temps d'un appel | vers l'IdP du proxy |
| état du SSO serveur (nonce, PKCE) | non | Redis | 300 s, usage unique | le proxy | non |

- Le seul flux nouveau est l'envoi du jeton du proxy à l'IdP du cluster ; il
  est documenté dans la page de sécurité et activé par l'opérateur.
- Les métriques ne portent ni utilisateur ni jeton (labels `cluster` et
  `result` seulement).

---

## 10. Operability

### 10.1 Observability

- **Metrics** : `proxyauth_token_exchange_total{result}` avec `result` ∈
  `exchanged`, `cache_hit`, `refused`, `unavailable` ; histogramme
  `proxyauth_token_exchange_duration_seconds`. Pas de label `cluster` pour
  borner la cardinalité ; le cluster est dans l'audit.
- **Logs and audit trail** : `token_exchanged` (cible `audit`) à chaque
  échange réel (pas sur un cache hit), avec utilisateur, `ns`, `cluster`,
  résultat. S'aligne sur le schéma de la RFC 0008.
- **Alerts** : aucune livrée ; la doc propose `rate(…{result="unavailable"})`
  au-dessus de 0 pendant 5 min.

### 10.2 Failure modes

| Failure | Detected by | Behaviour (fail open / closed) | Recovery |
| --- | --- | --- | --- |
| IdP du cluster injoignable ou lent (> 5 s) | erreur réseau, délai | **fermé** : `503`, pas compté en fail2login | automatique au retour de l'IdP |
| IdP refuse (`invalid_grant`, `unauthorized_client`) | statut 400/401 de l'IdP | **fermé** : `401` « token exchange refused », pas compté en fail2login (le jeton a déjà été validé à la porte 4) | corriger la config IdP ; `status.warnings` aide |
| réponse sans `access_token` ou trop grosse | parsing | **fermé** : `502` | idem |
| client d'échange mal configuré (secret absent) | avertissement de statut | fermé (l'IdP refuse) | corriger la CRD |
| IdP du proxy injoignable au rafraîchissement | erreur réseau | `503`, pas compté en fail2login ; `get-token` échoue avec ce message | automatique ; sinon nouvelle connexion SSO |
| refresh token refusé (session expirée) | `invalid_grant` de l'IdP | `400`, compté en fail2login ; `get-token` demande une nouvelle connexion | `kubectl-proxyauth login --server-url …` |

Toujours fermé : un échec d'échange ne relaie jamais le jeton du proxy à
l'apiserver.

### 10.3 Performance and capacity

- Un échange ajoute un aller-retour à l'IdP (Keycloak local : quelques
  dizaines de ms, à mesurer sur le banc en phase 4) une fois par durée de
  cache ; les requêtes suivantes ne coûtent qu'une recherche en mémoire.
- Mémoire : environ 2 Kio par (utilisateur, cluster) actif, borné.

---

## 11. Other concerns

- **Accessibility** : le front perd une étape (pas de login par cluster) ; le
  bouton existant change de destination, pas de nouveau composant.
- **Internationalisation** : une chaîne nouvelle dans le front (« connexion
  par échange ») dans chaque locale livrée.
- **Compliance** : réduit le nombre de secrets longue durée sur les postes
  (contrôle d'accès ISO 27001 A.5.17) ; l'audit couvre chaque émission.
- **Licensing** : inchangé.
- **Documentation** : `.docs/content/docs/` (sécurité, CRD, configuration de
  Keycloak et de Dex pour l'échange), le guide CLI.

---

## 12. Alternatives considered

| Alternative | Why rejected |
| --- | --- |
| Le proxy émet ses propres jetons (B) | une clé de signature qui ouvre tous les clusters, et une configuration d'apiserver par cluster, impossible sur certains managés |
| Impersonation (C) | un droit d'impersonation par cluster détenu par le proxy ; sa compromission ouvre tous les clusters |
| Rendre le jeton échangé au client (`/oauth/token` en RFC 8693) | garde un jeton par cluster côté client : le problème de §2.2 reste |
| Cache dans Redis, partagé par les réplicas | met des jetons de cluster dans Redis, ses dumps et ses sauvegardes, pour économiser au plus N−1 échanges |
| Dex `audience:server:client_id:` (cross-client trust) | Dex seulement, se décide au login, et donne un jeton multi-audience que tous les clusters accepteraient |
| Connexion SSO du CLI directement chez l'IdP du proxy, avec un client public dédié | l'opérateur enregistre un client de plus et deux mappers d'audience (vers `OIDC_AUDIENCE` pour que le proxy l'accepte, vers le client d'échange) ; la voie médiée n'en demande aucun (décision 10) |
| Garder le jeton collé sans rafraîchissement | expire en 5 minutes avec un Keycloak par défaut : l'échange ne servirait à rien (décision 8) |
| Ne rien faire | dix logins SSO pour dix clusters, dix jetons sur chaque poste |

---

## 13. Rollout and compatibility

- **Default behaviour** : inchangé ; `upstream_auth` absent ou dans un autre
  mode que `TokenExchange`.
- **Migration** : activer l'échange sur un cluster rend invalides les jetons
  de cluster déjà distribués pour lui (décision 1) ; les utilisateurs refont
  `kubectl-proxyauth login clusters`, qui écrit le nouveau contexte.
  Réversible : désactiver l'échange rétablit l'ancien chemin.
- **Operator prerequisites** : chez l'IdP du cluster, un client confidentiel
  avec l'échange activé ; pour Keycloak, le jeton du proxy doit porter ce
  client dans son `aud` (mapper d'audience sur le client du dashboard).
- **CLI** : `login --server-url --token` reste disponible ; la connexion SSO au
  serveur est un ajout, qui demande la redirection du proxy (`OIDC_REDIRECT_URL`)
  déjà enregistrée chez son IdP.
- **Rollback** : repasser en `mode: Passthrough` ; rien n'est persisté (cache
  en mémoire).
- **Feature flag** : `upstream_auth.mode` par cluster, et
  `upstreamAuth.allowedModes` du chart pour l'installation (RFC 0014).

---

## 14. Test plan

- **Unit** (`libs/server/crd`) : règles CEL et `validate()` de §4.3 ;
  `warnings()`.
- **Unit** (`libs/server/common/src/token_exchange.rs`) : formulaire envoyé
  (paramètres présents ou omis), parsing de réponse, limite de taille, calcul
  du TTL (min des trois bornes).
- **Integration** (`libs/server/api/tests/token_exchange.rs`, wiremock pour
  les deux IdP et l'apiserver) : succès, cache hit, IdP en panne (`503`),
  IdP qui refuse (`401`).
- **Proof** : §6.P (`tests/integration/tests/rfc0010.rs`).
- **Security** (`libs/server/api/tests/token_exchange.rs`), un test par ligne :
  - T1 `a_cluster_token_is_refused_on_an_exchange_cluster`
  - T2 `a_non_member_never_reaches_the_idp` (wiremock : 0 appel)
  - T3 `the_exchanged_token_never_reaches_the_client`
  - T4 `two_users_never_share_an_exchanged_token`
  - T5 `client_authorization_headers_are_all_replaced`
  - T6 `a_rate_limited_caller_never_reaches_the_idp`
  - T7 `an_exchange_writes_one_audit_event`
  - T8 `a_foreign_issuer_without_connector_warns`
  - T9 `the_cache_never_outlives_max_cache_seconds`
  - T10 `an_exchange_failure_never_relays_the_proxy_token`
  - T11 `only_a_validated_token_is_sent_to_the_exchange`
  - T12 `a_refused_refresh_counts_toward_a_ban`
- **Garde-fous** (RFC 0015) : la variante `UpstreamAuthMode::TokenExchange` ne
  compile pas sans son bras dans `libs/server/api/tests/variant_coverage.rs`,
  qui nomme son test ; la route éventuelle a sa ligne dans `authz_matrix.rs`.
- **Audit** : `task audit` vert (aucune dépendance nouvelle).
- **Existing suites** : `http_contract`, `proxy_security`, `proxy_upgrade`,
  envtest et `rfc0003`, `rfc0006`, `rfc0007` inchangés : un cluster sans
  échange se comporte comme avant.

---

## 15. Decisions and open questions

### Resolved

Tranchées au `/grill-me` du 2026-10-18.

| # | Question | Decision |
| --- | --- | --- |
| 1 | Un cluster en échange accepte-t-il aussi un jeton du cluster ? | **Non : un seul mode par cluster.** Deviner le mode depuis l'`iss` non vérifié du jeton ajoute un chemin de validation et une ambiguïté exploitable. |
| 2 | Où vit le jeton échangé ? | **En mémoire seulement, par réplica.** Redis exposerait des jetons de cluster pour économiser au plus N−1 échanges (§12). |
| 3 | Release | ~~**0.3.0**~~ ~~**0.4.0**~~ **0.5.0** (révisée le 2026-10-23 par [PLAN.md](PLAN.md) : la 0.3.0 est partie sans elle ; puis le 2026-10-24 : la 0012 passe avant, et la 0010 est relue face au mode `Agent` avant d'être codée), après la RFC 0008, dont `token_exchanged` reprend le schéma, et après la RFC 0011. |
| 4 | Quel client échange ? | **Le client du cluster** (`oidc_provider.client_id`) : il marche aussi en fédération ; le mapper d'audience Keycloak est documenté. |
| 5 | `max_cache_seconds` par défaut | **300 s** (la durée d'un jeton d'accès Keycloak par défaut), 3 600 s au plus. |
| 6 | Où se place l'échange | **Après toutes les portes**, juste avant le relais : une requête refusée ne coûte aucun appel à l'IdP. |
| 7 | Échecs de l'échange | **503** (injoignable), **401** (refus), **502** (illisible) ; aucun ne compte pour fail2login ; le jeton du proxy n'est jamais relayé. |
| 8 | Durée de vie du jeton du proxy côté CLI | **Dans la RFC** : une connexion SSO au serveur avec refresh token (§6.5). Le jeton collé expire en 5 minutes avec un Keycloak par défaut. |
| 9 | Audit et métrique | **`token_exchanged`** sur un échange réel seulement ; `proxyauth_token_exchange_total{result}` sans label `cluster`. |
| 10 | Connexion SSO du CLI au serveur | **Médiée par le proxy**, avec son client : une seule audience, aucune configuration nouvelle chez l'IdP ; rafraîchissement par `/api/v1/auth/refresh`. |
| 11 | Preuve sur le banc | **Un second Dex fédéré** (même image), RBAC par groupes ; Keycloak vérifié à la main, hors suite ; la faiblesse d'audience de Dex devient T11. Faisabilité vérifiée sur les sources de Dex v2.46.0. |
| 12 | Protection du rafraîchissement | **Throttling non scopé et fail2login** (un `invalid_grant` compte, un IdP injoignable non) ; refresh token dans le corps ; `server_login` et `token_refreshed` audités. |
| 13 | `offline_access` | **Non par défaut** : le refresh token suit la session SSO ; l'opérateur l'ajoute à `OIDC_SCOPES` s'il le veut. |
| 14 | `login --server-url --token` | **Gardé**, sans rafraîchissement (scripts, CI). |
| 15 | Phases | **Quatre** (§16). |

### Still open

Aucune.

---

## 16. Implementation phases

| Phase | Content |
| --- | --- |
| 1 | Serveur, l'échange : CRD (`token_exchange`, CEL, warnings), `common::token_exchange` et son cache, branchement dans `redirect` et les trois relais, `token_exchanged`, métriques, tests unitaires, d'intégration et de sécurité (T1 à T11). Utile seul : un opérateur peut l'activer et utiliser un jeton du proxy obtenu à la main. |
| 2 | Serveur, la session : `/api/v1/auth/{login,callback,refresh}` (§6.5), `server_login` et `token_refreshed`, T12 ; `visible_clusters.token_exchange` ; `swagger.json` et clients régénérés. |
| 3 | Clients : connexion SSO au serveur et rafraîchissement dans `get-token` (CLI), `login clusters` sans SSO pour un cluster en échange, kubeconfig à plugin dans le front. |
| 4 | Banc : Dex B fédéré, `AuthenticationConfiguration` de k3s, personas, `rfc0010.rs`, lignes de `MATRIX.md`, retrait de `PENDING` ; documentation (sécurité, CRD, Keycloak et Dex). |

Le contrôle (portes avant l'échange, refus fermés) arrive dans la même phase
que la surface : la phase 1 ne relaie jamais un jeton échangé sans eux.

---

## 17. Revision against the tree (2026-10-31)

Livrée sur la branche `rfc/0010-token-exchange` (PR #332), en quatre
commits, un par phase. Phases 1 et 2 vérifiées en processus (22 tests :
`libs/server/api/tests/{token_exchange,server_auth}.rs`, `authz_matrix`,
`openapi_contract`, `variant_coverage`) ; phase 3 par les tests unitaires du
CLI (189, dont 12 nouveaux) et du front ; la preuve §6.P (`rfc0010.rs`) ne
tourne que sur le banc de la CI : son résultat s'ajoute ici au premier
passage.

### Écarts au texte, avec leur raison

1. **La session SSO du CLI revient au proxy, pas au CLI.** §6.5 faisait
   rediriger l'IdP vers le listener loopback. Dex n'accepte pas de loopback
   sur un client public qui déclare des `redirectURIs` (celui du dashboard en
   a). L'IdP rappelle donc `<API_CLUSTER_OIDC_BASE_REDIRECT_URL>/api/v1/auth/callback`
   (une URI de plus à enregistrer sur le client du dashboard, documentée), le
   proxy échange le code, puis remet au loopback un **code à usage unique**
   (60 s, Redis) que le CLI échange par PKCE à une **quatrième route**,
   `POST /api/v1/auth/token` : le schéma du serveur OAuth médié
   (`cluster/auth/oauth`). Aucun jeton ne passe dans une URL. Les jetons de la
   session vivent au plus 60 s dans Redis, comme ceux de l'OAuth médié (§9).
2. **`TokenExchange` n'est pas un mode « de confiance ».**
   `UpstreamAuthMode::is_trusted` reste `FrontProxy | Issuer` (préfixes,
   sonde, `claim_mappings`) ; `replaces_token()` (tout sauf `Passthrough`)
   range le cluster sous la clé Redis de la décision 42 de la 0014 et exige
   `OIDC_AUDIENCE_VALIDATION=enforce` (T11). La sonde de confiance ne
   s'applique pas : l'apiserver croit l'IdP du cluster.
3. **Résultat `unreadable`** ajouté aux métriques et à l'audit (§10.1 n'en
   avait que quatre) : la réponse illisible (`502`) se distingue d'un refus.
4. **Avertissements** : en plus de `status.warnings`, une condition d'avis
   `TokenExchangeAtRisk` (catalogue de la 0011).
5. **Deux entrées de trousseau par serveur** (jeton d'accès, puis
   `<url>::session` avec le refresh token et l'expiration) au lieu d'une :
   le gestionnaire d'identifiants de Windows borne un secret à 2 560 octets.
   Les deux écritures ne sont pas atomiques.
6. **`login --server-url` sans `--token` lance toujours le SSO** : l'invite
   de saisie du jeton serveur disparaît (un script passe `--token`). `get` et
   `login clusters` rafraîchissent aussi la session, pas seulement
   `get-token`. `get-token` reconnaît un cluster en échange au drapeau
   `token_exchange` que `login` écrit dans la configuration du CLI : le
   kubeconfig du front demande donc un `login` du cluster, sans navigateur.
7. **Banc** : `offline_access` ajouté à `OIDC_SCOPES` (Dex ne rend un refresh
   token qu'à cette portée, décision 13) ; `UNSCOPED_MAX_FAILED_LOGINS=1000`
   pour que le point 9 voie le compteur ; Traefik `allowEmptyServices` pour
   que Dex B arrêté réponde `503` (point 6) ; le point 8 passe par
   `/api/v1/auth/refresh` en HTTP plutôt que par l'expiration d'un jeton de
   Dex A, partagé avec les autres suites ; `rfc0010` dans la passe série.
8. **Après la revue de code** : un `408` ou `429` de l'IdP du cluster est une
   indisponibilité (`503`), pas un refus ; la clé du cache inclut aussi
   `scope` et `connector_id` ; un succès de cache n'attend plus la discovery ;
   le verrou par clé est libéré même si la requête est annulée ; seul
   `invalid_grant` sur `/auth/refresh` compte pour le ban (`invalid_client`
   donne `503`) ; un callback `?error=` revient au CLI en `access_denied` ;
   le CLI sérialise le renouvellement de session (verrou de fichier) pour ne
   pas dépenser deux fois un refresh token qu'un IdP fait tourner.

### Menaces (§7)

T1 à T7, T10 et T11 : `libs/server/api/tests/token_exchange.rs` (noms de §14,
T5 dans `kube_redirect/upstream.rs`) ; T8 :
`crd::upstream_auth::token_exchange` ; T9 : `common::token_exchange` ; T12 :
`libs/server/api/tests/server_auth.rs`. Sur le banc : points 3, 4, 7 et 9.

### Supply chain (§8)

`secrecy` 0.10.3 et `form_urlencoded` 1.2.2 deviennent dépendances directes
de `common` (déjà dans `Cargo.lock`) ; aucune image nouvelle (Dex B est
l'image de Dex).
