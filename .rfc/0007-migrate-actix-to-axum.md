# RFC 0007 — Migrer le serveur HTTP d'actix-web vers axum (hyper 1)

| Field       | Value                                                                  |
| ----------- | ---------------------------------------------------------------------- |
| Status      | Draft                                                                  |
| Short       | actix-web vers axum                                                    |
| Settles     | Le serveur (API, OAuth, proxy, upgrades) passe d'actix-web 4 à axum 0.8 sur hyper 1, à comportement constant, et le tunnel d'upgrade utilise `hyper::upgrade` au lieu de HTTP écrit à la main |
| Author      | Max Batleforc <maxleriche.60@gmail.com>                                |
| Co-author   | Claude Opus 5.5                                                        |
| Created     | 2026-10-17                                                             |
| Supersedes  | —                                                                      |
| Issue       | — (à créer)                                                            |
| Blocks      | RFC 0004 (le contrôle de flux s'écrit sur le tunnel hyper)             |
| Touches     | `apps/server`, `libs/server/api/**` (57 fichiers), `libs/server/api/tests/**` (23 fichiers, 110 tests), `Cargo.toml`, `apps/server/Cargo.toml`, `libs/server/api/Cargo.toml`, docs développeur |
| Risk        | high                                                                   |

---

## 1. Summary

Le serveur ProxyAuthK8S tourne sur actix-web 4. Pour les upgrades (exec,
attach, port-forward, watch), actix-http ne rend au handler les octets qui
suivent le handshake que pour `Upgrade: websocket` (`upgrade.rs:195`). Le
proxy compense en ouvrant lui-même la socket TCP/TLS vers l'apiserver, en
**écrivant la requête HTTP à la main** (`upgrade/connect.rs`, 255 lignes), en
**relisant la réponse à la main** (`upgrade/response.rs`, 486 lignes) et en
reliant les deux par un canal `mpsc` (`upgrade/tunnel.rs`). Cette RFC migre
tout le serveur vers **axum 0.8** sur **hyper 1**, la pile déjà présente via
`kube` et `reqwest`, et déjà utilisée par `weebo-kit-runtime` (axum 0.8 +
axum-server 0.8). Les upgrades passent par `hyper::upgrade::on` des deux
côtés. La migration est **iso-fonctionnelle** : mêmes routes, mêmes réponses,
même OpenAPI, SPDY toujours refusé.

### Before / after

```text
# aujourd'hui
HttpServer (actix) + bind_rustls_0_23          main.rs:81,133
middlewares : wrap_fn request-id, TracingLogger (tracing-actix-web),
              HttpMetrics (Transform maison), Compress, actix-cors
OpenAPI     : utoipa-actix-web + utoipa-scalar (feature actix-web)
upgrade     : TcpStream + TLS + requête écrite à la main + 101 relu à la main + mpsc
proxy std   : Payload -> mpsc -> reqwest::Body ; réponse re-parsée en types actix
tests       : 110 #[actix_web::test], init_service ; proxy_upgrade = vrai HttpServer

# avec cette RFC
axum::Router + axum-server (tls-rustls)        comme weebo-kit-runtime
middlewares : tower-http (request-id, trace, compression, cors) + HttpMetrics en tower Layer
OpenAPI     : utoipa-axum + utoipa-scalar (feature axum)
upgrade     : hyper::upgrade::on(req) côté client, client hyper + upgrade::on(res) côté apiserver,
              copy_bidirectional enveloppé par le filtre
proxy std   : axum::body::Body -> reqwest::Body (même crate http 1.x, pas de re-parsing)
tests       : tower::ServiceExt::oneshot ; proxy_upgrade = vrai serveur axum
```

```mermaid
flowchart LR
    subgraph T["aujourd'hui"]
        C1["client"] --> AX["actix-http"]
        AX -- "Payload" --> M["mpsc"]
        M --> RAW["socket TLS + HTTP écrit à la main"]
        RAW --> A1["apiserver"]
    end
    subgraph W["avec cette RFC"]
        C2["client"] --> HY["hyper (axum)"]
        HY -- "upgrade::on" --> F["filtre (ports, exec)"]
        F -- "client hyper + upgrade::on" --> A2["apiserver"]
    end
```

Le diagramme montre ce qui disparaît : le HTTP écrit à la main sur une socket
brute, qui est la partie la plus exposée au smuggling.

---

## 2. Motivation

1. **Le tunnel d'upgrade réimplémente HTTP.** `serialize_upgrade_request`
   (`upgrade/connect.rs:115`) écrit la requête, `read_upgrade_response_headers`
   (`upgrade/response.rs`) relit le `101`. Chaque garde (refus de corps
   `upgrade.rs:214`, en-têtes hop-by-hop, taille des en-têtes) est à maintenir
   à la main, sur le chemin qui donne exec dans les conteneurs.
2. **actix limite le protocole d'upgrade.** Seul `Upgrade: websocket` est
   rendu au handler ; le contrôle de flux de la RFC 0004 et le filtre de ports
   doivent passer par un `Payload` actix relié à un `mpsc`
   (`tunnel.rs:51-63`), avec deux `actix_web::rt::spawn`.
3. **Deux piles HTTP coexistent.** actix-http côté serveur, hyper 1 côté client
   (`kube`, `reqwest`). `standard.rs:240-290` convertit statut et en-têtes de
   `reqwest` vers les types actix, en jetant ceux qui ne sont pas UTF-8.
4. **Écosystème divergent du kit.** `weebo-kit-runtime` (RFC 0005) sert son
   webhook avec axum 0.8 et axum-server 0.8 ; une future adoption de ses crates
   (télémétrie, santé) suppose tower/axum.

---

## 3. Goals / non-goals

**Goals**

- Plus aucune dépendance `actix-web`, `actix-cors`, `tracing-actix-web`,
  `utoipa-actix-web`.
- Iso-fonctionnel : mêmes routes, mêmes statuts, mêmes en-têtes, même
  OpenAPI (diff de `swagger.json` vide), mêmes événements d'audit et
  métriques.
- Le tunnel d'upgrade utilise `hyper::upgrade` des deux côtés ; plus de
  sérialisation HTTP à la main.
- Les 110 tests existants sont portés et passent, plus une suite de
  non-régression HTTP indépendante du framework écrite **avant** la
  migration.

**Non-goals**

- **Relayer SPDY** : toujours refusé.
- **Changer une route, un format de réponse ou un défaut** : tout changement
  visible est un bug de migration.
- **mTLS entrant** : il n'existe pas aujourd'hui (`server_config.rs:59`,
  `with_no_client_auth`) ; le seul certificat client est sortant, vers
  l'apiserver (`kube_redirect/tls.rs:90`), et il est conservé tel quel.
- **Adopter les crates du kit** : RFC dédiée, après celle-ci.
- **Implémenter la RFC 0004** : elle s'appuie sur ce résultat.

---

## 4. User-facing design

### 4.1 Configuration

Aucune option ajoutée ni modifiée : `server_config.https`, les chemins PEM,
`CorsOrigins` (SameOriginOnly / Any / List), le timeout d'arrêt et le port
gardent leur sens.

### 4.2 Behaviour rules

- **Correspondance des middlewares** :

  | actix (ordre actuel, `main.rs:82-120`) | axum / tower |
  | --- | --- |
  | `wrap_fn` qui recopie `RequestId` en `x-request-id` (:97-116) | `tower_http::request_id::{SetRequestIdLayer, PropagateRequestIdLayer}` |
  | `TracingLogger::<OtelRootSpanBuilder>` (`root_span.rs:25`, lit `traceparent`) | `tower_http::trace::TraceLayer` avec un `MakeSpan` qui lit `traceparent` |
  | `HttpMetrics` (Transform maison, `metrics.rs:98,123`, `match_pattern()` pour `http.route`) | `Layer` maison, `MatchedPath` pour `http.route` |
  | `Compress` (:120) | `tower_http::compression::CompressionLayer` ; les flux qui posent `content-encoding: identity` restent non compressés |
  | `actix-cors` (:82-92) | `tower_http::cors::CorsLayer`, mêmes origines, `max_age` 3600 |

- **Routes** : `/{ns}/{cluster}/{path:.*}` devient `/{ns}/{cluster}/{*path}`.
  Le chemin capturé doit être le **même texte brut** qu'aujourd'hui (pas de
  décodage supplémentaire), parce que les matchers décodent une fois eux-mêmes.
- **Extracteurs** : `PeerAddr` et `req.peer_addr()` deviennent
  `ConnectInfo<SocketAddr>` ; `web::Data<State>` devient `State<AppState>` ;
  `impl FromRequest for User` (`model/user.rs:130`) devient `FromRequestParts`.
- **Erreurs** : `AuthError::into_actix_error` (`helper/mod.rs:34`) devient
  `impl IntoResponse`.
- **Futures `Send`** : axum exige des handlers `Send`. Le futur de
  `User::from_request` est aujourd'hui boxé sans `Send` (`user.rs:132`) ; tout
  état non `Send` doit être corrigé, pas contourné par `spawn_local`.

### 4.3 Validation

| Condition | Rationale |
| --- | --- |
| `swagger.json` généré diffère | la règle Weebo veut l'OpenAPI comme contrat ; tout écart est un bug |
| la suite de non-régression HTTP (§6.1) diffère | iso-fonctionnel |
| un test existant est supprimé au lieu d'être porté | la couverture ne doit pas baisser pendant la migration |

---

## 5. Architecture

### 5.1 Tunnel d'upgrade

```mermaid
sequenceDiagram
    participant K as kubectl
    participant S as axum handler
    participant H as client hyper
    participant A as apiserver
    K->>S: GET …/exec (Upgrade: websocket)
    S->>S: politiques (ports, verbes, exec) avant tout
    S->>H: requête construite par http::Request (en-têtes filtrés)
    H->>A: TLS (rustls, certificat client sortant inchangé)
    A-->>H: 101
    H-->>S: hyper::upgrade::on(response)
    S-->>K: 101 (mêmes en-têtes que l'upstream)
    S->>S: hyper::upgrade::on(request)
    S->>S: copy_bidirectional, client -> upstream via filtre
```

La requête vers l'apiserver est construite par `http::Request` et sérialisée
par hyper : un corps déclaré, un en-tête invalide ou une ligne de requête
malformée sont refusés par la bibliothèque, pas par du code maison.

### 5.2 Invariant

Comme hyper sérialise et parse les deux handshakes, aucun octet du client
n'atteint l'apiserver hors d'une requête HTTP bien formée construite par le
proxy ; et comme le filtre enveloppe le sens client → upstream du flux
upgradé, aucune donnée de session n'atteint l'apiserver sans passer par lui.

---

## 6. Detailed design

### 6.1 Avant de migrer : figer le comportement

- Nouvelle suite `libs/server/api/tests/http_contract.rs` qui lance le
  **vrai binaire serveur** sur un port éphémère (comme `proxy_upgrade.rs`) et
  ne parle qu'en HTTP (reqwest, client websocket) : statuts, en-têtes, corps
  pour chaque route, CORS (preflight et origines), compression, `x-request-id`,
  `traceparent`, métriques `/management/metrics`, refus d'upgrade.
- Elle passe sur actix **avant** toute migration ; elle est la preuve de §6.P.

### 6.2 Serveur

- `apps/server/src/main.rs` : `axum::Router`, état partagé, couches tower,
  `axum_server::bind_rustls` (même `rustls::ServerConfig` que
  `server_config.rs:42-61`) ou `axum::serve` sans TLS. Arrêt gracieux avec le
  même délai (`main.rs:132`) via `axum_server::Handle`.
- Le `tokio::select!` avec le contrôleur (`main.rs:149-158`) reste.
- `utoipa-axum` : `OpenApiRouter` pour `/management`, `/api/v1`, `/clusters` ;
  `gen_openapi()` (`lib.rs:56-69`) rebâti sur le même routeur.
- `utoipa-scalar` avec la feature `axum`, même URL `/api/docs`.

### 6.3 Proxy standard

- `standard.rs` : `axum::body::Body` devient `reqwest::Body` sans `mpsc`
  (`Body::into_data_stream`) ; la réponse `reqwest` est rendue avec ses types
  `http` 1.x, sans re-parsing (`standard.rs:240-290` se réduit au filtrage
  hop-by-hop).
- `virtual_redirect/watch.rs` : même reframage ligne par ligne, sur
  `Body::from_stream`.

### 6.4 Upgrade

- `upgrade/connect.rs` et `upgrade/response.rs` sont remplacés par un client
  hyper (`hyper_util::client::legacy` ou `hyper::client::conn::http1` sur la
  connexion TLS existante de `tls.rs`) ; `serialize_upgrade_request` disparaît.
- `upgrade/tunnel.rs` : `hyper::upgrade::on` des deux côtés, puis
  `tokio::io::copy_bidirectional` avec un `AsyncRead` enveloppé par le filtre
  (`PortForwardFilter`, puis `ExecChannelFilter` de la RFC 0004).
- SPDY : toujours refusé, mais **après** les politiques (RFC 0004 Q15).

### 6.5 Tests

- 110 `#[actix_web::test]` deviennent `#[tokio::test]` avec
  `tower::ServiceExt::oneshot` sur le `Router` ; les helpers `harness/`,
  `envtest_support/`, `virtual_api_support/` sont portés en premier.
- `proxy_upgrade.rs` démarre un vrai serveur axum.

### 6.P Proof against a real client

Clients : `kubectl` 1.34 (exec, attach, cp, port-forward, logs `-f`, `get
-w`), le plugin `kubectl-proxyauth` du dépôt, un navigateur sur le front, et
`curl`. Contre k3d (`task kube3d:*`).

1. `http_contract.rs` passe à l'identique avant et après la migration.
2. `task gen:swagger-file` puis `git diff --exit-code swagger.json` : vide.
3. `kubectl exec -it`, `kubectl attach`, `kubectl cp` (aller et retour),
   `kubectl port-forward` (port autorisé) et `kubectl logs -f` fonctionnent ;
   `kubectl get pods -w` reçoit les événements ligne par ligne.
4. **Refus** : `kubectl port-forward` sur un port hors politique donne le 403
   avec sa raison, y compris après le repli SPDY ; l'upstream enregistreur n'a
   reçu aucune connexion.
5. **Refus** : une requête d'upgrade avec `Content-Length: 10` donne 400, et
   aucun octet n'est écrit vers l'upstream (anti-smuggling, désormais garanti
   par hyper).
6. **Refus** : un `Upgrade: SPDY/3.1` conforme aux politiques donne le 400
   « SPDY is not supported ».
7. Preflight CORS depuis une origine non listée : refusé comme aujourd'hui.
8. `/management/metrics` expose `http_server_*` avec le même `http.route`.
9. Débit `kubectl exec … -- cat /dev/zero | head -c 1G` : au moins égal à
   l'actuel.

**Deliberately untouched**, so reviewers do not go looking :

- `libs/server/crd` : aucune dépendance HTTP.
- La logique d'authentification, d'OAuth et de throttle Redis
  (`throttle.rs`) : seules leurs signatures changent.
- Le certificat client sortant vers l'apiserver (`tls.rs:90`).

---

## 7. Security

**Rating:** high. Toute la surface HTTP authentifiée et le proxy sont
réécrits ; une régression peut ouvrir un accès. En contrepartie, le HTTP écrit
à la main disparaît.

### 7.1 Threat model

```mermaid
flowchart LR
    U["client (non fiable)"] -- "TLS, HTTP/1.1, upgrade" --> S["axum / hyper"]
    S -- "client hyper, TLS, certificat client" --> A["apiserver"]
    S -- "reqwest" --> I["IdP"]
```

| # | Threat (STRIDE) | Where | Mitigation | Residual risk |
| --- | --- | --- | --- | --- |
| T1 | Tampering : smuggling vers l'apiserver sur le chemin d'upgrade | handshake upstream | hyper sérialise la requête ; refus de corps conservé ; §6.P point 5 | faible (bibliothèque) |
| T2 | Elevation : un middleware de sécurité oublié (CORS, throttle) | `Router` | suite `http_contract.rs` écrite avant ; CORS testé (point 7) | faible |
| T3 | Elevation : chemin capturé décodé différemment (`{*path}`) | routage | test : le chemin brut transmis aux matchers est identique (encodages, `%2F`, `..`) | faible |
| T4 | Info disclosure : en-têtes hop-by-hop ou d'authentification relayés | proxy standard et upgrade | filtrage existant (`forwarded.rs`) porté tel quel, testé | aucun |
| T5 | DoS : limites de taille (en-têtes, corps) différentes | hyper / axum | `DefaultBodyLimit` et limites d'en-têtes hyper réglées sur les valeurs actix actuelles, testées | faible |
| T6 | Spoofing : IP client mal lue (throttle, `X-Forwarded-For`) | `ConnectInfo` | `forwarded::throttle_client_ip` porté, test sur `ConnectInfo` | aucun |

### 7.2 Authentication and authorisation

Mêmes contrôles, mêmes points d'appel : le passage de `FromRequest` à
`FromRequestParts` ne change pas la logique (`model/user.rs`). Prouvé par les
tests `oauth_*`, `oidc_*` et `proxy_security.rs` portés.

### 7.3 Input handling and injection

Le parsing HTTP passe d'actix-http à hyper, deux implémentations maintenues
et fuzzées. Le proxy ne parse plus aucune réponse HTTP lui-même.

### 7.4 Secrets and cryptography

Même `rustls::ServerConfig`, même configuration TLS sortante. Aucun secret
nouveau.

### 7.5 Abuse and resource exhaustion

Limites portées (T5). Les tunnels restent bornés (`UPGRADE_CHANNEL_CAPACITY`
devient le tampon de `copy_bidirectional`).

### 7.6 Existing defences that still hold

Throttle Redis, fail2login, garde anti-traversée, refus de SPDY, refus de corps
sur upgrade, filtrage des en-têtes d'authentification amont.

---

## 8. Supply chain

| Dependency / image / tool | Version | Why this one | Licence | Known CVEs (`task audit`) | Maintenance |
| --- | --- | --- | --- | --- | --- |
| `axum` | 0.8.9 | routeur sur hyper 1, utilisé par le kit | MIT | à vérifier | équipe tokio ; dernière release 2026-04-14 ; 0.x : une mineure peut casser |
| `axum-server` | 0.8 (feature `tls-rustls-no-provider`) | TLS serveur, comme `weebo-kit-runtime` | MIT | à vérifier | actif |
| `tower-http` | dernière 0.6 | CORS, trace, compression, request-id | MIT | à vérifier | équipe tokio |
| `utoipa-axum` | 0.3.0 (2026-09-22) | OpenAPI (règle Weebo) | MIT/Apache-2.0 | à vérifier | actif |
| `utoipa-scalar` | 0.4, feature `axum` | UI Scalar (règle Weebo) | MIT/Apache-2.0 | — | actif |
| `hyper`, `hyper-util`, `http`, `tower` | déjà dans l'arbre (kube, reqwest) | upgrade, client | MIT | — | équipe tokio |
| **Retirés** : `actix-web`, `actix-cors`, `tracing-actix-web`, `utoipa-actix-web` et leurs transitives (`actix-http`, `actix-router`, `actix-server`…) | — | — | MIT/Apache-2.0 | — | — |

- **Build et release** : image back inchangée de forme ; SBOM plus petite (une
  seule pile HTTP).
- **Updates** : Dependabot cargo ; axum 0.x suivi comme une dépendance à
  mineures cassantes (lecture du CHANGELOG à chaque mineure).

---

## 9. Data and privacy

| Data | Personal? | Where stored | Retention | Who can read it | Leaves the instance? |
| --- | --- | --- | --- | --- | --- |
| aucune nouvelle | — | — | — | — | — |

Les logs, l'audit et les métriques gardent les mêmes champs ; aucun nouveau
champ n'est collecté.

---

## 10. Operability

### 10.1 Observability

- **Metrics** : mêmes noms et mêmes labels (`http.route` via `MatchedPath`).
- **Logs and audit trail** : mêmes événements ; le span racine lit toujours
  `traceparent`.
- **Alerts** : aucune nouvelle.

### 10.2 Failure modes

| Failure | Detected by | Behaviour | Recovery |
| --- | --- | --- | --- |
| route ou en-tête différent | `http_contract.rs` en CI | build rouge | corriger avant merge |
| régression en production | métriques 4xx/5xx | — | rollback de l'image |
| futur non `Send` | compilation | build rouge | corriger l'état partagé |

### 10.3 Performance and capacity

Performances équivalentes attendues ; le tunnel perd un saut `mpsc` et deux
`spawn`. Mesure §6.P point 9, notée dans la PR.

---

## 11. Other concerns

- **Accessibility** : pas d'UI.
- **Internationalisation** : messages inchangés.
- **Compliance** : SBOM réduite.
- **Licensing** : MIT/Apache-2.0 des deux côtés.
- **Documentation** : guide développeur (architecture du serveur, tests),
  `CLAUDE.md` si une convention de test change.

---

## 12. Alternatives considered

| Alternative | Why rejected |
| --- | --- |
| Garder actix | le HTTP écrit à la main reste sur le chemin d'exec ; la RFC 0004 s'empile dessus |
| hyper seul, sans axum | pas de routeur ni d'extracteurs, pas d'intégration utoipa : la règle Weebo « Utoipa + Scalar » devient intenable |
| Servir seulement le proxy avec hyper, l'API avec actix | deux serveurs, deux ports ou un proxy interne ; plus complexe que la migration |
| Pingora | conçu pour un reverse proxy L7 sans API applicative ; ni OpenAPI ni extracteurs ; pile différente de kube |
| Migration incrémentale route par route dans le même binaire | deux frameworks sur un même port n'est pas possible sans proxy interne |

---

## 13. Rollout and compatibility

- **Default behaviour** : identique (c'est le critère d'acceptation).
- **Migration** : aucune pour les opérateurs ni les clients.
- **Operator prerequisites** : aucun.
- **Rollback** : image précédente.
- **Feature flag** : non. Une seule release porte la migration, après une
  période de test sur k3d et sur un cluster de recette.

---

## 14. Test plan

- **Contrat** : `http_contract.rs` écrit et vert sur actix avant la
  migration, inchangé après.
- **Unit et intégration** : les 110 tests portés ; aucun supprimé.
- **Proof** : §6.P.
- **Security** : un test négatif par ligne T1 à T6.
- **Audit** : `task audit` vert, `cargo machete` sans dépendance actix
  restante.
- **OpenAPI** : diff vide de `swagger.json`.

---

## 15. Decisions and open questions

### Resolved

| # | Question | Decision |
| --- | --- | --- |
| 0 | Migrer, et vers quoi ? | **axum sur hyper 1**, avant la RFC 0004 (grill de la RFC 0004, Q12). |
| 1 | SPDY | **Toujours refusé**, après les politiques. |

### Still open

1. **Client hyper pour l'upgrade** : `hyper_util::client::legacy` (pool,
   plus simple) ou `hyper::client::conn::http1` sur la connexion TLS de
   `tls.rs` (contrôle fin, pas de pool pour des sessions longues) ?
   Recommandation : `conn::http1`.
2. **TLS serveur** : `axum-server` (comme le kit) ou accepteur
   `tokio-rustls` maison ? Recommandation : `axum-server`.
3. **Ordre avec la RFC 0006** : 0006 avant (indépendante du framework, ferme
   T7 plus tôt) ou après ? Recommandation : avant.
4. **Découpage des PRs** : une PR par couche (contrat, serveur et
   middlewares, proxy standard, upgrade, tests) sur une branche d'intégration,
   ou une seule PR ? Le binaire ne peut pas mélanger les deux frameworks.
5. **Période de recette** : combien de temps sur un cluster de recette avant
   la release ?
6. **Release** : dans la 0.2.0 (avec les RFC 0001 et 0002) ou dans la
   suivante ? Recommandation : la suivante, pour ne pas cumuler deux refontes.

---

## 16. Implementation phases

| Phase | Content |
| --- | --- |
| 1 | `http_contract.rs` sur actix (aucun changement de code serveur) |
| 2 | Helpers de test et extracteurs portés ; `Send` corrigé là où il manque |
| 3 | Serveur, middlewares tower, OpenAPI (`swagger.json` identique) |
| 4 | Proxy standard et watch |
| 5 | Upgrade sur `hyper::upgrade`, suppression de `connect.rs` et `response.rs` |
| 6 | Retrait d'actix, `cargo machete`, docs |

Les phases 2 à 6 vivent sur une branche d'intégration : le binaire ne peut pas
servir les deux frameworks à la fois. La phase 1 est mergée seule sur `main`.
