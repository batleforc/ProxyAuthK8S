# RFC 0008 — Trace d'audit : couverture, schéma et sortie garantie

| Field       | Value                                                                  |
| ----------- | ---------------------------------------------------------------------- |
| Status      | Implemented (2026-10-18)                                               |
| Short       | Trace d'audit                                                          |
| Settles     | Chaque décision de sécurité du serveur (accès proxifié, refus de politique, authentification, remise de jetons OAuth, bannissement, fin de session) produit un évènement d'audit d'un schéma versionné, que le niveau de log ne peut pas faire taire |
| Author      | Max Batleforc <maxleriche.60@gmail.com>                                |
| Co-author   | Claude Opus 5.5                                                        |
| Created     | 2026-10-17                                                             |
| Supersedes  | —                                                                      |
| Issue       | — (à créer)                                                            |
| Depends on  | RFC 0003 et 0006 (évènements de politique livrés) ; la phase 4 attend la RFC 0007 (tunnel hyper) |
| Blocks      | RFC 0004 (ses évènements `exec_open`/`exec_closed` s'écrivent sur ce schéma) |
| Touches     | `libs/server/trace/src/lib.rs`, `libs/server/api/src/{root_span.rs,metrics.rs}`, `libs/server/api/src/cluster/redirect/{audit.rs,throttle.rs}`, `…/kube_redirect.rs`, `…/kube_redirect/{upgrade.rs,upgrade/tunnel.rs}`, `libs/server/api/src/cluster/auth/{login.rs,callback.rs,oauth/*.rs}`, `tests/integration/`, `mise.toml`, `.docs/content/docs/security.mdx` |
| Risk        | medium                                                                 |

> Document de référence (environ 4 900 mots) : §1 à §4 suffisent pour en
> discuter ; §5 à §16 servent à l'implémenter et à le relire.

---

## 1. Summary

Aujourd'hui, l'audit se limite au chemin de proxy. `AuditContext::emit`
(`redirect/audit.rs:135`) écrit un évènement `proxied request` par requête
proxifiée, plus les décisions de ports et de verbes des RFC 0003 et 0006. Tout
le reste n'en laisse aucune trace : la connexion d'un utilisateur, la remise
des jetons de l'IdP par le serveur OAuth intégré, le bannissement d'un client, la durée
d'une session exec. Et un `RUST_LOG=warn` éteint l'audit sans un mot.

Cette RFC fait de l'audit un flux à part, en quatre points :

- **Toujours émis** : une couche dédiée, insensible à `RUST_LOG`.
- **Un format** : lignes JSON d'un schéma versionné (`audit_schema: 1`), avec
  `request_id`, `trace_id` et l'adresse du client.
- **Couvrant** chaque décision de sécurité : authentification, jetons,
  bannissements, refus de politique, fin de session.
- **Consignant le refus amont** d'un upgrade websocket, que le repli SPDY de
  kubectl masque à l'utilisateur ; le `400` SPDY lui dit comment le voir.

### Before / after

```text
# aujourd'hui (RUST_LOG=info, LOG_FORMAT=text)
INFO audit: proxied request user=alice groups=dev ns=default cluster=c verb=GET path=/api/v1/namespaces/dev/pods response_status=200 latency_ms=12
(RUST_LOG=warn : rien)        (POST /oauth/token qui remet les jetons : rien)
(kubectl exec sur un pod Pending) error: unable to upgrade connection: this proxy only tunnels websocket upgrades; SPDY is not supported…

# avec cette RFC (quel que soit RUST_LOG ou LOG_FORMAT)
{"audit_schema":1,"event":"request","outcome":"allowed","request_id":"…","trace_id":"…","client_ip":"10.0.0.7","user":"alice","groups":["dev"],"ns":"default","cluster":"c","http_method":"GET","k8s_verb":"list","path":"/api/v1/namespaces/dev/pods","status":200,"latency_ms":12}
{"audit_schema":1,"event":"oauth_token_issued","outcome":"allowed","client_ip":"10.0.0.7","user":"alice","ns":"default","cluster":"c","grant_type":"authorization_code","redirect_uri":"http://127.0.0.1:8000/",…}
{"audit_schema":1,"event":"request","outcome":"denied",…,"path":"/api/v1/namespaces/dev/pods/web/exec","status":400,"upstream_reason":"pod web does not have a host assigned"}
(kubectl exec sur un pod Pending) error: … SPDY is not supported …; if a websocket attempt failed just before, kubectl -v=6 shows the upstream reason
```

```mermaid
flowchart LR
    subgraph T["aujourd'hui"]
        R1["redirect()"] --> E1["évènement tracing<br/>cible audit"]
        A1["login, oauth/token,<br/>bannissement"] -.->|"rien"| X1["∅"]
        E1 --> F1{"RUST_LOG ?"}
        F1 -->|"warn"| X2["perdu"]
        F1 -->|"info"| O1["stdout texte ou JSON"]
    end
    subgraph W["avec cette RFC"]
        R2["redirect()"] --> E2["AuditEvent v1"]
        A2["login, oauth/token,<br/>bannissement, fin de session"] --> E2
        E2 --> L["couche audit dédiée<br/>filtre fixe audit=info"]
        L --> O2["stdout, une ligne JSON"]
        E2 --> S["évènement de span OTel"]
    end
```

Le schéma montre les deux pertes d'aujourd'hui : des décisions qui n'émettent
rien, et un filtre global qui jette ce qui est émis.

---

## 2. Motivation

1. **Un niveau de log éteint l'audit.** Le filtre `env_filter` construit depuis
   `RUST_LOG` (`trace/src/lib.rs:290`) s'applique à tout le `Registry`
   (`:300`), donc aussi à la cible `audit`. Un opérateur qui passe
   `RUST_LOG=warn` pour calmer ses logs perd tout l'audit, sans avertissement.
   `security.mdx` (« Journal d'audit ») suggère bien `RUST_LOG=warn,audit=info`,
   mais rien ne l'impose.
2. **La remise de jetons n'est pas tracée.** Le serveur OAuth intégré
   (`cluster/auth/oauth/authorize.rs:60`, `callback.rs:51`, `token.rs:69`)
   échange un code contre les jetons de l'IdP, puis les remet au client local
   (`IssuedCode`, `oauth/model.rs:47`). L'identité n'est connue qu'au callback
   (`claims.subject()`, `callback.rs:281`, un `info!` hors audit), et
   `IssuedCode` ne la garde pas. `token.rs` ne journalise que des erreurs
   (`:103,109`). Une compromission ne laisse donc aucune trace de qui a reçu
   des jetons, ni quand.
3. **L'authentification et les bannissements non plus.** `cluster_login`
   (`auth/login.rs:40`) et `callback_login` (`auth/callback.rs:46`) n'écrivent
   qu'un `info!` hors audit. Le bannissement décidé par
   `throttle::record_auth_failure` (`redirect/throttle.rs:114`) ne produit
   rien : seul le `429` de la requête suivante apparaît, comme une requête
   ordinaire.
4. **Les évènements ne se recoupent pas.** Le champ `verb`
   (`redirect/audit.rs:142`) contient la méthode HTTP, alors que
   `emit_verb_denied` ajoute `k8s_verb` : un `GET` peut être un `get`, un
   `list`, un `watch` ou un `create` (RFC 0006). L'évènement ne porte ni
   `request_id`, ni `trace_id`, ni l'adresse du client. Le texte n'en donne que
   les spans, et le JSON seulement via `with_span_list`
   (`trace/src/lib.rs:229-230`).
5. **Une session n'a pas de fin.** L'évènement d'une session upgradée part au
   `101` (`upgrade.rs:329`). La fin de la session (durée, octets, qui a fermé)
   n'est jamais écrite (`upgrade/tunnel.rs:53-59`). La RFC 0004 le constate
   pour exec (motivation 3) ; c'est vrai de tout tunnel (port-forward, attach,
   watch).
6. **Le repli SPDY masque le refus amont.** Constaté le 2026-10-17 avec
   kubectl 1.37 (RFC 0006 §17), et de nouveau le 2026-10-18 sur le banc de la
   RFC 0009 (kubectl 1.37.1, serveur axum) : l'apiserver refuse le websocket
   d'un `kubectl exec` sur un pod `Pending` (« pod pending does not have a
   host assigned »), et le proxy relaie ce refus (`refused_upgrade_response`,
   `upgrade.rs:262`). kubectl retente alors en SPDY et n'affiche que le `400`
   du proxy, « SPDY is not supported » (`upgrade.rs:145`) ; la vraie raison
   n'apparaît qu'avec `kubectl -v=6`. L'audit, lui, ne garde que le statut.

---

## 3. Goals / non-goals

**Goals**

- Un évènement d'audit pour chaque décision de sécurité du serveur (§4.2),
  dans un schéma unique et versionné.
- Un audit qui ne dépend ni de `RUST_LOG` ni de `LOG_FORMAT`.
- Chaque évènement porte `request_id`, `trace_id`, `client_ip`, `http_method`
  et, quand il est calculable, `k8s_verb`.
- La raison d'un refus amont d'upgrade est consignée ; le `400` SPDY dit à
  l'utilisateur comment la voir.
- Un évènement `session_closed` pour chaque tunnel. La RFC 0004 y ajoute
  `argv` et ses canaux.

**Non-goals**

- **Stockage inaltérable (WORM, signature, chaînage de hachés)** : c'est le
  rôle du collecteur de l'opérateur. Le serveur garantit l'émission, pas la
  conservation.
- **Intégration SIEM spécifique** (Splunk HEC, syslog) : la sortie est une
  ligne JSON sur stdout, que tout collecteur sait router.
- **Exporteur OTLP logs** : les évènements restent aussi des évènements de
  span OTel, comme aujourd'hui (§12).
- **Audit des appels du contrôleur vers les clusters** (réconciliation,
  reachability) : ils ne portent aucune identité d'utilisateur ;
  `status.error` et les logs du contrôleur les couvrent.
- **Audit du corps des requêtes ou de la query string** : ils peuvent porter
  des secrets (arguments d'exec, sélecteurs) ; l'argv d'exec relève de la
  RFC 0004 et de son `mask_exec_secrets`.
- **Interrupteur pour couper l'audit** : un audit qu'on éteint par
  configuration ne prouve rien. On filtre côté collecteur.
- **Remplacer l'audit de l'apiserver cible** : il reste la source pour ce que
  l'apiserver a exécuté. Le proxy trace ce qu'il a décidé.

---

## 4. User-facing design

### 4.1 Configuration

Aucune option nouvelle.

- Les évènements d'audit sortent **toujours** : couche dédiée à filtre fixe
  `audit=info`, toujours au format JSON, quel que soit `LOG_FORMAT`. La
  couche de logs ordinaire exclut la cible `audit`, pour qu'il n'y ait pas de
  doublon.

### 4.2 Behaviour rules

**Évènements** (`event`), et quand ils sont émis :

| `event` | Émis par | Quand |
| --- | --- | --- |
| `request` | `redirect()` | une requête proxifiée reçoit sa réponse (remplace `proxied request`) |
| `policy_decision` | `redirect()`, `check_handshake`, `tunnel` | décision de ports (RFC 0003) ou de verbes (RFC 0006) : `policy`, `decision`, `port`, `reason` |
| `auth_login` | `cluster_login` | redirection vers l'IdP d'un cluster |
| `auth_callback` | `callback_login` | retour de l'IdP : `outcome` allowed (identité résolue) ou denied (nonce, code ou jeton invalide) |
| `oauth_authorize` | `oauth::authorize` | demande d'autorisation acceptée ou refusée : `redirect_uri` (loopback, sans query) ; il n'y a pas de `client_id`, le serveur n'enregistre aucun client |
| `oauth_callback` | `oauth::callback` | retour de l'IdP : `user` (sujet du jeton d'identité vérifié) en cas de succès, raison en cas d'échec (nonce, `at_hash`, échange) |
| `oauth_token_issued` / `oauth_token_denied` | `oauth::token` | jetons remis ou refusés : `user` (repris d'`IssuedCode`), `redirect_uri`, `grant_type`, `expires_in` ; raison du refus (`invalid_grant`, PKCE, `redirect_uri`) ; jamais un jeton ni le code |
| `client_banned` | `throttle::record_auth_failure` | un bannissement est posé ou prolongé : à chaque échec au-delà du seuil de `fail2login` (`ban_duration_for` rend une durée, `throttle.rs:138`) ; `failures`, `ban_seconds` (0 : permanent) |
| `session_closed` | `tunnel` | fin d'un tunnel : `duration_ms`, `bytes_in`, `bytes_out`, `close_reason` (client, upstream, filter, error) |

**Champs communs**, présents sur chaque évènement :

- `audit_schema` (`1`), `event`, `outcome` (`allowed`, `denied`, `error`) ;
- `request_id` (l'uuid v4 que `root_span` pose dans `x-request-id` ; jamais un
  identifiant fourni par le client), `trace_id` (celui du span OTel) ;
- `client_ip` : l'adresse calculée par `throttle_client_ip`, qui honore
  `TRUSTED_PROXY_COUNT`, comme pour les bans ;
- `user` (`-` si inconnu), `groups` (tableau), `ns`, `cluster`.

Sur `request` et `policy_decision` s'ajoutent `http_method`, `k8s_verb` (si
`request_info` aboutit), `path` (sans query), `status` et `latency_ms`.
`verb` reste un alias de `http_method` jusqu'en v1.0.0, puis disparaît, avec
annonce dans le CHANGELOG. Pour une API virtuelle, `k8s_verb` et `path` sont
ceux de la requête d'origine, et `upstream_path` / `upstream_k8s_verb` ceux
de la requête réécrite, les deux étant vérifiés (RFC 0006 T5).

**Contenu interdit**, quel que soit l'évènement : l'en-tête `Authorization`,
tout jeton ou code (d'accès, de rafraîchissement, d'autorisation, `id_token`),
le `client_secret`, la query string, les corps. Les champs sont une liste
fermée, construite par `AuditEvent`, jamais un dump de la requête.

**Refus amont d'un upgrade** : quand l'upstream répond autre chose que `101`,
le corps est déjà lu en entier (`refused_upgrade_response`, limité à 1 Mio).
Le champ `message` du `Status`, tronqué à 256 caractères, est consigné dans
`upstream_reason` de l'évènement `request` ; un corps qui n'est pas un
`Status` donne une raison absente. Le `400` « SPDY is not supported » gagne
une phrase fixe : `if a websocket attempt failed just before, kubectl -v=6
shows the upstream reason`. Pas de cache : le message est le même pour tous
(décision 12).

**Cas sans rien de nouveau** : un serveur sans requête n'émet rien. Un cluster
sans `security_config` émet `request`, comme aujourd'hui. Un appel au
dashboard (`/api/v1/clusters`) n'émet rien : il ne fait que lire la liste des
clusters visibles, déjà filtrée par groupe (décision 8).

### 4.3 Validation

Rien n'est refusé : il n'y a aucune entrée nouvelle à valider.

| Condition | Behaviour | Surfaced in |
| --- | --- | --- |
| `RUST_LOG` exclut la cible `audit` (par exemple `warn`) | l'audit sort quand même | un `warn!` au démarrage : « RUST_LOG no longer filters the audit target » |
| `LOG_FORMAT=text` | logs en texte, audit en JSON sur le même stdout | doc (`security.mdx`) : filtrer sur `"audit_schema"` |

---

## 5. Architecture

### 5.1 Émission

```mermaid
flowchart TD
    D["décision : redirect, auth, oauth, throttle, tunnel"] --> B["AuditEvent::new(event, ctx)<br/>champs fermés"]
    B --> E["tracing::info!(target: audit, …)"]
    E --> LA["couche audit<br/>Targets audit=info, JSON"]
    E --> LO["couche OTel<br/>évènement de span"]
    E --> LF["couche fmt ordinaire<br/>filtre RUST_LOG, audit exclu"]
    LA --> OUT["stdout : 1 ligne JSON"]
    LF -.->|"jamais"| OUT2["doublon"]
```

Un évènement emprunte toujours la couche audit, parce que son filtre est un
`Targets` fixe. `RUST_LOG` ne règle que la couche ordinaire et le filtre OTel,
qui ne voit plus la cible `audit` comme filtrable.

### 5.2 Repli SPDY

```mermaid
sequenceDiagram
    participant K as kubectl
    participant P as proxy
    participant A as apiserver
    K->>P: GET …/exec (Upgrade: websocket)
    P->>A: handshake
    A-->>P: 400 Status "pod web does not have a host assigned"
    P->>P: audit request upstream_reason=…
    P-->>K: 400 (relayé)
    K->>P: POST …/exec (Upgrade: SPDY/3.1)
    P->>P: politiques puis NotWebsocket
    P-->>K: 400 "SPDY is not supported…; if a websocket attempt failed just before, kubectl -v=6 shows the upstream reason"
```

L'opérateur trouve la raison dans l'audit ; l'utilisateur sait où la
chercher. Aucun état ne relie les deux requêtes.

### 5.3 Invariant

Toutes les sorties passent par `AuditEvent`, dont les champs sont une liste
fermée. Aucun appelant ne peut donc y mettre un en-tête ou un corps arbitraire,
ni un jeton. Et la couche audit a son propre filtre fixe : aucune valeur de
`RUST_LOG` ne peut retirer un évènement de stdout.

---

## 6. Detailed design

### 6.1 `libs/server/trace`

- `start_tracing` ajoute une troisième couche :
  `fmt::layer().json().flatten_event(true).with_current_span(false)` filtrée
  par `Targets::new().with_target("audit", Level::INFO)`.
- La couche ordinaire reçoit `.with_filter(filter_fmt.add_directive("audit=off"))`.
- Le filtre global `env_filter` ne s'applique plus au `Registry` entier, mais
  par couche : ordinaire et OTel. Sinon il couperait la couche audit en amont.
  La couche OTel garde exactement les directives d'aujourd'hui (celles de
  `base_filter`, dont la mise en sourdine des crates de transport), pour que
  le volume de spans exportés ne change pas.
- Avertissement au démarrage si la valeur de `RUST_LOG` aurait exclu `audit`.

### 6.2 `libs/server/api`

- `redirect/audit.rs` : `AuditEvent` (champs de §4.2, `serde`-free :
  construit des champs `tracing`), `AuditEvent::emit()`. `AuditContext`
  garde ses champs et gagne `request_id`, `trace_id`, `client_ip`,
  `k8s_verb` ; `emit`, `emit_port_decision` et `emit_verb_denied` deviennent
  des appels à `AuditEvent`. `sanitize_audit_field` reste en défense en
  profondeur : plus aucune sortie texte ne reçoit la cible `audit`, et les
  évènements de span OTel sont des attributs structurés.
- `kube_redirect.rs` : `client_ip` (déjà calculé, `peer_id`) et `k8s_verb`
  (déjà calculé, `request_info`) passent dans `AuditContext`.
- `root_span.rs` : l'uuid de `x-request-id` est rangé dans les extensions de la
  requête, où `AuditContext` le lit.
- `upgrade.rs` : `refused_upgrade_response` rend aussi le `message` du
  `Status` amont, que `upgrade_redirect` consigne dans `upstream_reason` ;
  `UpgradeRefusal::NotWebsocket` gagne la phrase fixe.
- `auth/login.rs`, `auth/callback.rs`, `auth/oauth/{authorize,callback,token}.rs` :
  un `AuditEvent` par issue (accepté, refusé, erreur).
- `auth/oauth/model.rs` : `IssuedCode` gagne `subject` (et `groups` si le
  jeton d'identité en porte), posé au callback, pour que `token.rs` sache à
  qui il remet les jetons. Le code vit 60 s dans Redis (`CODE_TTL_SECONDS`),
  qui détient déjà les jetons eux-mêmes : le sujet n'y ajoute rien de
  sensible.
- `throttle::record_auth_failure` : `client_banned` quand le compteur franchit
  le seuil.
- `upgrade/tunnel.rs` (phase 4, sur le tunnel hyper de la RFC 0007) :
  compteurs d'octets dans les deux moitiés, `session_closed` à la fin de la
  dernière.

### 6.3 Docs

- `security.mdx` « Journal d'audit » réécrit : schéma v1, table des
  évènements, champs interdits, exemple de filtre collecteur, alias `verb`.

### 6.P Proof against a real client

Banc de la RFC 0009 (k3d, chart, Dex), `tests/integration/tests/rfc0008.rs`,
une fonction `pN_sujet` par point, toujours à travers le proxy. Clients : le
kubectl épinglé du banc (`mise.toml`, 1.35 ; le constat du point 7 tient aussi
avec 1.37.1), le plugin `kubectl-proxyauth`
du dépôt, kubelogin (`mise.toml`) pour le serveur OAuth intégré, et `reqwest`
pour les requêtes forgées. Les lignes d'audit sont lues dans les logs du pod
serveur (`observe::audit_lines`).

1. Serveur lancé avec `RUST_LOG=warn` : `kubectl get pods` produit **une**
   ligne `{"audit_schema":1,"event":"request","k8s_verb":"list",…}`, et
   aucune ligne `info` ordinaire.
2. `kubectl get pods -w` : `k8s_verb` vaut `watch`, `http_method` `GET` ;
   `request_id` est l'en-tête `x-request-id` reçu par le client, et
   `trace_id` est présent et non vide.
3. `kubectl proxyauth login` : la sortie contient `auth_login` puis
   `auth_callback` (`outcome: allowed`) pour cet utilisateur. `kubectl
   oidc-login get-token` contre le serveur OAuth intégré : `oauth_authorize`,
   `oauth_callback`, puis `oauth_token_issued`, ces deux derniers avec le même
   `user`.
   **Refus** : une recherche du jeton émis (lu dans le cache de kubelogin) sur
   toute la sortie du serveur ne trouve rien.
4. **Refus** : `/oauth/token` avec un code invalide produit
   `oauth_token_denied` et un `400`.
5. **Refus** : `fail2login.max_failed_logins: 2`, puis trois requêtes avec un
   jeton faux : une ligne `client_banned` avec `client_ip` et `ban_seconds`,
   puis des `request` en `429`.
6. **Refus** : `kubectl delete pod web` sous une règle en lecture seule : une
   ligne `policy_decision` (`policy: verb`, `decision: denied`, `k8s_verb:
   delete`) et une ligne `request` en `403`.
7. `kubectl exec` sur un pod `Pending` : kubectl affiche `…SPDY is not
   supported…; if a websocket attempt failed just before, kubectl -v=6 shows
   the upstream reason`, et la ligne `request` du `GET` porte
   `upstream_reason` avec la raison de l'apiserver.
8. `kubectl port-forward pod/web 8080` puis l'arrêt du client donne
   `session_closed` avec `close_reason: client` et des octets non nuls
   (phase 4).
9. Un `kubectl get` avec `Authorization: Bearer <jeton>` : le jeton
   n'apparaît dans aucune ligne, ni d'audit, ni de log.

**Deliberately untouched**, so reviewers do not go looking :

- Les décisions elles-mêmes (ports, verbes, groupes, bans) : seule leur
  trace change.
- `virtual_redirect` : il émet déjà via `AuditContext`, et hérite du schéma.
- Le contrôleur (`libs/server/controller`) : aucune identité d'utilisateur
  (§3).

---

## 7. Security

**Rating:** medium. Aucun contrôle d'accès ne change, mais l'audit est la
défense contre la répudiation, et il touche à des données sensibles (jetons,
identité, adresse).

### 7.1 Threat model

```mermaid
flowchart LR
    U["client (non fiable)"] -- "en-têtes, chemin, jeton" --> P["serveur proxy"]
    P -- "lignes JSON stdout" --> C["collecteur de l'opérateur"]
    C --> S["stockage d'audit"]
    I["IdP"] -- "claims user, groups" --> P
```

| # | Threat (STRIDE) | Where | Mitigation | Residual risk |
| --- | --- | --- | --- | --- |
| T1 | Repudiation : audit perdu par un `RUST_LOG` restrictif | `trace` | couche audit à filtre `Targets` fixe (§6.1) ; avertissement au démarrage | aucun |
| T2 | Repudiation : jetons remis sans trace | `oauth/callback.rs`, `oauth/token.rs` | `oauth_callback` et `oauth_token_issued` / `_denied` à chaque issue ; le sujet suit le code dans `IssuedCode` | aucun |
| T3 | Info disclosure : jeton, code ou secret dans l'audit | `AuditEvent` | champs fermés ; aucun en-tête, query ou corps ; test qui cherche le jeton dans toute la sortie | `upstream_reason` reprend un message de l'apiserver (borné à 256 caractères) ; accepté |
| T4 | Tampering : faux évènement par un champ contrôlé (`user`, `path`, `upstream_reason`) | sortie | JSON : le sérialiseur échappe CR/LF ; la couche texte ordinaire exclut `audit` ; `sanitize_audit_field` gardé en défense en profondeur | aucun |
| T5 | *retirée* : elle visait l'indice SPDY, abandonné (décision 12) | — | — | — |
| T6 | Spoofing : `client_ip` falsifiée par `X-Forwarded-For` | `throttle_client_ip` | `TRUSTED_PROXY_COUNT` existant, défaut 0 | celui déjà accepté pour les bans (`security.mdx`, risques acceptés) |
| T7 | DoS : saturation par le volume d'audit | stdout | un évènement par décision, pas par octet | un stdout bloqué bloque les requêtes (décision 7, §10.2) ; accepté |
| T8 | Spoofing : `request_id` imposé par le client pour brouiller les recoupements | `root_span` | l'uuid est généré par le proxy ; un `x-request-id` entrant est ignoré | aucun |

Tests négatifs : T1 → §6.P point 1 ; T2 → point 3 ; T3 → points 3 et 9 ;
T4 → `user` contenant `\n{"audit_schema":1}` reste un seul champ JSON ;
T8 → une requête avec `x-request-id: forged` est auditée avec l'uuid du
proxy.

### 7.2 Authentication and authorisation

Aucune surface nouvelle : pas de route, pas de jeton.

### 7.3 Input handling and injection

Le seul parseur nouveau lit le message d'un `Status` amont, déjà lu en entier
(`refused_upgrade_response`, borné à 1 Mio) : le JSON est décodé, le champ
`message` tronqué à 256 caractères, et tout échec donne une raison absente.
Pas de fuzz dédié : `serde_json` sur un corps borné.

### 7.4 Secrets and cryptography

Aucun secret n'est ajouté. La règle « jamais un jeton » est portée par la liste
fermée des champs et prouvée par §6.P points 3 et 9.

### 7.5 Abuse and resource exhaustion

Un évènement par décision, déjà le cas pour `request`.

### 7.6 Existing defences that still hold

`sanitize_audit_field`, l'exclusion de la query string, `TRUSTED_PROXY_COUNT`,
le refus de SPDY après les politiques (RFC 0003 R13).

---

## 8. Supply chain

| Dependency / image / tool | Version | Why this one | Licence | Known CVEs (`task audit`) | Maintenance |
| --- | --- | --- | --- | --- | --- |
| aucune crate nouvelle | — | `tracing-subscriber` (feature `json`) et `serde_json` sont déjà dans l'arbre | — | — | — |
| kubelogin (banc seulement, `mise.toml`) | dernière version | client OAuth réel pour §6.P point 3 | Apache-2.0 | aucune connue | int128/kubelogin, publié régulièrement |

- **Build et release** : inchangés.
- **Updates** : sans objet.

---

## 9. Data and privacy

| Data | Personal? | Where stored | Retention | Who can read it | Leaves the instance? |
| --- | --- | --- | --- | --- | --- |
| `client_ip` (nouveau) | **oui** | stdout, collecteur | politique de l'opérateur | opérateurs | selon le collecteur |
| `user`, `groups` (déjà là) | oui | idem | idem | idem | idem |
| `redirect_uri` (OAuth, loopback) | non | idem | idem | idem | idem |
| `subject` dans `IssuedCode` | oui (identifiant) | Redis, avec les jetons | 60 s (`CODE_TTL_SECONDS`) | le serveur | non |
| `upstream_reason` | peut nommer un pod ou un namespace | stdout | politique de l'opérateur | opérateurs | selon le collecteur |
| `request_id`, `trace_id` | non | idem | idem | idem | vers le collecteur OTel déjà configuré |

- L'adresse est déjà dans les clés de bannissement (Redis) : l'audit la
  rend lisible au même public, les opérateurs. Elle sert à enquêter
  (répudiation, bannissements) ; la minimisation porte sur le reste (pas de
  user-agent, pas de query).
- Effacement et export : par le collecteur, sur `user` et `client_ip`.

---

## 10. Operability

### 10.1 Observability

- **Metrics** : `proxyauth_audit_events_total{event, outcome}`. Les deux
  labels sont bornés (l'énumération de §4.2) ; ni utilisateur, ni chemin.
- **Logs and audit trail** : l'objet de la RFC. Une ligne JSON par évènement,
  table de §4.2.
- **Alerts** : aucune par défaut. Une hausse de `client_banned` ou
  d'`oauth_token_denied` relève d'une règle de l'opérateur, documentée comme
  exemple.

### 10.2 Failure modes

| Failure | Detected by | Behaviour | Recovery |
| --- | --- | --- | --- |
| stdout bloqué (collecteur arrêté, pipe plein) | latence des requêtes | l'écriture bloque la requête (fermé : pas de décision sans trace, décision 7) ; c'est l'écriture synchrone d'aujourd'hui | relancer le collecteur |
| corps du refus amont illisible | `upstream_reason` absent | le refus est relayé comme aujourd'hui | aucune |
| `RUST_LOG` excluant `audit` | avertissement au démarrage | audit émis quand même | aucune |

### 10.3 Performance and capacity

Une sérialisation JSON par décision, contre une ligne texte aujourd'hui. Le
coût est à mesurer par le banc de débit de la RFC 0007 (§6.P point 9), avec
un seuil de 5 % de latence ajoutée au p99.

---

## 11. Other concerns

- **Accessibility** : pas d'UI.
- **Internationalisation** : la phrase du `400` SPDY et l'audit sont en
  anglais, comme les autres messages d'API.
- **Compliance** : traçabilité ISO 27001 A.8.15 (journalisation) et A.5.28
  (preuves) ; NIS2 art. 21 (journalisation des accès). Le schéma versionné
  devient la pièce produite à l'audit.
- **Licensing** : inchangé.
- **Documentation** : `security.mdx` (Journal d'audit, risques acceptés),
  CHANGELOG (alias `verb`, JSON forcé).

---

## 12. Alternatives considered

| Alternative | Why rejected |
| --- | --- |
| Documenter `RUST_LOG=…,audit=info` (état actuel) | repose sur chaque opérateur ; le défaut d'une erreur est un audit vide |
| Exporteur OTLP logs (`opentelemetry-appender-tracing`) | une dépendance et un second pipeline pour ce que stdout fait déjà ; les évènements restent des évènements de span. Réévaluable si un opérateur en a besoin |
| Fichier d'audit dédié avec rotation | état sur disque dans un conteneur sans volume ; la rotation et la rétention reviennent au collecteur |
| Audit en texte quand `LOG_FORMAT=text` | deux formats à maintenir pour une sortie lue par des machines |
| Indice SPDY : la raison amont gardée 10 s dans un cache local et ajoutée au `400` SPDY (proposition initiale) | un cache par réplique, donc best effort derrière un répartiteur sans affinité, une option de configuration et une menace (ancienne T5) pour un message ; la phrase fixe et l'audit couvrent le besoin (décision 12) |
| Indice SPDY partagé via Redis | une écriture Redis par refus amont pour un message |
| Ne pas relayer le refus amont (répondre `101` puis fermer) | contredit l'invariant d'upgrade (seul un `101` amont ouvre un tunnel) |
| Un champ `verb` réutilisé pour le verbe k8s | casse silencieusement les requêtes des collecteurs ; l'alias puis le retrait en v1.0.0 sont annoncés |

---

## 13. Rollout and compatibility

- **Default behaviour** : l'audit passe en JSON sur stdout, même en
  `LOG_FORMAT=text`, et sort même avec `RUST_LOG=warn`. Le message
  `proxied request` devient `event: request`, `response_status` devient
  `status`, et `verb` reste un alias de `http_method`.
- **Migration** : les requêtes des collecteurs passent de
  `message="proxied request"` à `event="request"`. Le CHANGELOG l'annonce
  (`feat!:`) avec la table de correspondance.
- **Operator prerequisites** : aucune.
- **Rollback** : image précédente ; la couverture et le format reviennent,
  sans état persistant.
- **Feature flag** : aucun (§3).

---

## 14. Test plan

- **Unit** (`redirect/audit.rs`) : chaque `event` produit exactement ses
  champs ; `verb` = `http_method` ; aucun champ hors liste.
- **Unit** (`trace`) : un subscriber avec `RUST_LOG=warn` capture quand même
  un évènement `audit` dans la couche audit, et aucun dans la couche
  ordinaire.
- **Unit** (`upgrade.rs`) : `message` d'un `Status` extrait et tronqué à
  256 caractères ; un corps qui n'est pas un `Status` ne donne rien.
- **Integration** (`tests/audit_trail.rs`, nouveau) : capture de la couche
  audit ; un cas par ligne de §4.2, dont OAuth (`oauth_as_*` existants
  réutilisés) et `client_banned` (`proxy_throttle.rs`).
- **Integration** (`proxy_upgrade.rs`) : le `400` SPDY porte la phrase fixe.
- **Proof** : §6.P.
- **Security** : un test négatif par ligne T1 à T8 (§7.1, T5 retirée).
- **Audit** : `task audit` vert, sans dépendance nouvelle.
- **Existing suites** : toutes. Les tests qui lisent `proxied request` sont
  portés sur `event: request`.

---

## 15. Decisions and open questions

### Resolved

Tranchées au `/grill-me` du 2026-10-18. Les décisions 1 à 6 étaient des
propositions de l'auteur ; la 5 a été renversée (12).

| # | Question | Decision |
| --- | --- | --- |
| 1 | Peut-on couper l'audit ? | **Non.** Un audit qu'une configuration éteint ne prouve rien ; le filtrage se fait au collecteur. |
| 2 | Format | **JSON toujours**, schéma versionné `audit_schema: 1`, même avec `LOG_FORMAT=text` ; le collecteur le reconnaît à `"audit_schema"`. |
| 3 | Sortie | **stdout**, en plus des évènements de span OTel existants ; pas de fichier ni d'exporteur OTLP logs (§12). |
| 4 | `client_ip` | **Ajoutée**, calculée comme pour les bans (`TRUSTED_PROXY_COUNT`) ; ni user-agent ni query. |
| 5 | Repli SPDY qui masque le refus amont | **Remplacée par la décision 12.** |
| 6 | Session close pour tous les tunnels ou seulement exec | **Tous**, en phase 4 (étape 12c du PLAN) ; la RFC 0004 ajoute ses champs propres à exec. |
| 7 | Échec d'écriture sur stdout | **Bloquer la requête.** C'est l'écriture synchrone d'aujourd'hui (`trace/src/lib.rs`, `std::io::stdout`) : rien à coder, aucune dépendance ; un proxy d'accès sans trace est le cas que la RFC ferme. Coût mesuré au banc de débit de la RFC 0007. |
| 8 | Le dashboard (`/api/v1/clusters`) audite-t-il ? | **Non** : lecture seule d'une liste déjà filtrée par groupe ; revu si le dashboard gagne des écritures. |
| 9 | Schéma et compatibilité | **`event: request`, `status`, `verb` alias de `http_method` jusqu'en v1.0.0**, annoncé en `feat!:` dans la 0.2.0. |
| 10 | Origine de `request_id` | **L'uuid du proxy** (`x-request-id` de `root_span`), jamais un identifiant fourni par le client (T8) ; `trace_id` vient du span OTel. |
| 11 | Où se fait la preuve | **Sur le banc de la RFC 0009** (`rfc0008.rs`), avec le kubectl du banc, kubelogin (`mise.toml`) et `observe::audit_lines`, plutôt que les binaires envtest du premier jet. |
| 12 | Indice SPDY | **Pas d'indice ni de cache** : `upstream_reason` dans l'audit (13) et une phrase fixe dans le `400` SPDY qui renvoie à `kubectl -v=6`. Le banc a confirmé le 2026-10-18 que kubectl 1.37.1 masque bien la raison ; un indice par réplique ne marcherait qu'une fois sur N. |
| 13 | `upstream_reason` | **Toujours consigné** sur `request` : `message` du `Status` amont, 256 caractères au plus. |
| 14 | Métrique | **`proxyauth_audit_events_total{event, outcome}`**, labels bornés ; pas d'alerte livrée, un exemple dans la doc. |

### Still open

Aucune.

---

## 16. Implementation phases

| Phase | Content |
| --- | --- |
| 1 | `AuditEvent` et schéma v1 ; couche audit dédiée insensible à `RUST_LOG` (contrôle de T1, livré avec le format) ; `request_id`, `trace_id`, `client_ip`, `http_method`/`k8s_verb` ; alias `verb` ; docs et CHANGELOG `feat!:` |
| 2 | Évènements d'authentification et OAuth (`auth_*`, `oauth_*`), `client_banned` (contrôles de T2 et T3) |
| 3 | `upstream_reason` et la phrase fixe du `400` SPDY (décisions 12 et 13) |
| 4 | `session_closed` sur le tunnel hyper de la RFC 0007 ; la RFC 0004 s'y appuie |

Les phases 1 à 3 sont indépendantes du framework HTTP : elles touchent
l'émission, pas le transport, et peuvent sortir avant la RFC 0007. La phase 4
attend son tunnel, pour ne pas écrire deux fois le comptage d'octets.

---

## 17. Revision against the tree (2026-10-18, phases 1 à 3)

Les phases 1 à 3 sont livrées (étape 12b du PLAN), puis la phase 4 le même jour
(étape 12c, voir la fin de cette section).

Ce qui diffère du texte, et pourquoi :

- **Pas de couche `tracing` dédiée (§6.1).** La couche JSON de
  `tracing-subscriber` n'écrit pas de tableaux : `groups` serait sorti en
  chaîne. `AuditEvent` construit donc l'objet JSON lui-même et l'écrit sur
  stdout, de façon synchrone (décision 7) ; aucun filtre ne s'interpose, ce
  qui tient la promesse de T1 plus directement qu'un filtre fixe. Un évènement
  `tracing` minimal (`event`, `outcome`) part vers le span OTel, et la couche
  de logs ordinaire exclut la cible `audit` (`audit=off`). Conséquence :
  l'avertissement de démarrage « RUST_LOG no longer filters the audit target »
  (§4.3) n'a plus d'objet et n'est pas écrit.
- **`request_id` et `client_ip` viennent d'un scope de requête.** `root_span`
  les pose dans un `task_local` tokio (`audit::scope`), lu par
  `AuditEvent::new` : aucun émetteur ne les transporte. Une tâche lancée à part
  (le tunnel de la phase 4) devra les recopier.
- **`trace_id` vaut `null` quand `RUST_LOG` coupe les spans `info`** : il n'y a
  alors pas de trace. Forcer le span racine changerait le volume exporté vers
  OTel, ce que §6.1 exclut. Le point 2 du banc, qui tourne en `RUST_LOG=warn`,
  vérifie la présence du champ, pas sa valeur.
- **`decision` prend les valeurs de la métrique** (`allowed`, `denied`,
  `node_proxy_denied`, `closed_by_filter`) sous `policy: port|verb`, au lieu de
  `port_allowed`, `port_denied`, `verb_denied`… : un seul vocabulaire pour la
  ligne et pour `proxyauth_policy_decisions_total`.
- **`outcome` d'une ligne `request`** : `allowed` dès que le proxy a laissé
  passer la requête, quel que soit le statut de l'amont (un refus d'upgrade
  relayé est `allowed`, avec `upstream_reason`) ; `denied` ou `error` pour un
  refus du proxy lui-même (4xx ou 5xx).
- **Les erreurs OAuth en redirection** (`302` vers le client avec `?error=`)
  sont classées par leur code, ramené à une liste fermée (`HandlerAudit`) :
  `server_error` est une `error`, les autres un refus.
- **`IssuedCode` porte `subject`**, pas `groups` : le jeton d'identité n'en
  porte pas toujours, et `/oauth/token` n'en a pas besoin pour nommer le
  destinataire. Un code émis avant la mise à jour n'a pas de `subject`, et sa
  ligne garde `user: "-"`.
- **Preuve du point 3** : kubelogin ne peut pas piloter le serveur OAuth
  médié, puisqu'il découvre l'IdP par `/.well-known/openid-configuration`,
  que le proxy n'expose pas. `oc login --web`, le client que la fonction
  imite, échoue aujourd'hui sur la racine `/clusters/{ns}/{cluster}`, qui n'est
  pas routée (« Seems you passed an HTML page »). `rfc0008::p3` prouve donc
  `auth_login` et `auth_callback` par la connexion SSO du plugin ;
  `oauth_token_issued` et l'absence de jeton (T2, T3) sont prouvés par
  `tests/audit_trail.rs`, et `oauth_token_denied` par `rfc0008::p4`. La
  compatibilité `oc login` est suivie par l'issue #277, et la doc ne la
  promet plus.

Preuves : `rfc0008::p1` à `p7` et `p9` verts sur le banc (kubectl 1.37.1 côté
poste, 1.35 épinglé en CI) ; `tests/audit_trail.rs` (5 tests) ; T4 et la
troncature de `upstream_reason` en tests unitaires de `audit.rs` ; T8 dans
`root_span.rs`. §8 : aucune crate nouvelle, kubelogin non ajouté.

### Phase 4 (2026-10-18)

`session_closed` est écrit à la fin de chaque tunnel (`upgrade/tunnel.rs`),
filtré ou non. Les deux flux sont enveloppés dans un `Counted` qui compte les
octets lus et note le premier côté arrivé en fin de flux : `close_reason` vaut
`filter` si le filtre de port-forward a coupé, `error` sur une erreur
d'entrée-sortie, sinon `client` ou `upstream` selon le premier qui a fermé.
`outcome` est `error` dans ce seul cas, `allowed` sinon (le refus du filtre a
déjà sa ligne `policy_decision`). La tâche du tunnel survit au handler : elle
reprend le scope de requête et le span, comme §17 l'avait prévu.

Preuves : `rfc0008::p8` sur le banc (port-forward arrêté par le client :
`close_reason: client`, octets non nuls), `proxy_upgrade.rs` (session tunnelée :
4 octets dans chaque sens, `client` ; port-forward SPDY refusé : `filter`), et
les tests unitaires de `tunnel.rs` (comptage et premier côté fermé).

