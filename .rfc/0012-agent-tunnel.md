# RFC 0012 — Mode agent : joindre un apiserver qui n'est pas exposé

| Field       | Value                                                                  |
| ----------- | ---------------------------------------------------------------------- |
| Status      | Implemented, phase 1 (2026-10-24, PR #294) : `ServiceAccountToken`, `tlsMode: inner`, sortie directe ; phases 2 à 4 à faire (§16) ; écarts au §17 |
| Short       | Mode agent (tunnel sortant)                                            |
| Settles     | Atteindre un cluster dont l'apiserver n'accepte aucune connexion entrante |
| Author      | Max Batleforc <maxleriche.60@gmail.com>                                |
| Co-author   | Claude Opus 5.5 (1M context)                                           |
| Created     | 2026-10-23                                                             |
| Supersedes  | —                                                                      |
| Touches     | phase 1 : `apps/agent/` (nouveau), `libs/agent/{proto,pki}/` (nouveaux), `libs/server/api/src/agent/` (nouveau : passerelle, écoute, enrôlement, CA, registre, présences, `CONNECT`), `libs/server/api/src/cluster/redirect/{kube_redirect.rs,audit.rs}` et `kube_redirect/{upstream.rs,upgrade/connect.rs}`, `libs/server/api/src/{metrics.rs,server.rs,model/user.rs}`, `libs/server/common/src/{agent_gateway.rs,config.rs,state.rs,state/redis.rs}`, `libs/server/crd/src/{service/agent.rs,status/mod.rs}`, `libs/server/crd_runtime/`, `libs/server/controller/` (`status.agent`, codes `AgentNoTunnel` et `AgentLimitCapped`), `libs/server/jwt_validator/` (`key_at`, `verify_with_key`), `apps/server/` (rôles), `deploy/chart/` (passerelle), `deploy/chart-agent/` (nouveau), `deploy/build/{agent,back}/Containerfile`, CRD générée (`deploy/crds.yaml`, `deploy/chart-crd/`, `deploy/crd/`), `fuzz/`, `.github/workflows/{build.yaml,release-prepare.yml}`, `.github/dependabot.yaml`, `cog.toml`, `Taskfile.yaml`, `.tasks/{lint,audit,integration}.yaml`, `.it/`, `tests/integration/`, `docs/content/docs/` |
| Risk        | high                                                                   |

> Document de référence (environ 7 500 mots) : §1 à §4 suffisent pour en
> discuter ; §5 à §16 servent à l'implémenter et à le relire.

---

## 1. Summary

Aujourd'hui, le proxy compose lui-même la connexion vers l'apiserver d'un
cluster : `Service::url_to_call` (`libs/server/crd/src/service/mod.rs`) donne
une URL, puis `connect_upgrade_stream`
(`kube_redirect/upgrade/connect.rs`) fait un `TcpStream::connect` et le client
`reqwest` de `upstream.rs` en fait autant. Un apiserver derrière un NAT, sur un
réseau privé ou sur un site edge est donc hors d'atteinte, à moins de l'exposer.
Cette RFC ajoute une troisième variante de `spec.service`, `Agent`, et un
binaire `proxyauth-agent` installé sur le cluster distant. L'agent ouvre une
connexion **sortante** vers le proxy (WebSocket, multiplexée par yamux, à la
manière du konnectivity-agent) ; le proxy y ouvre un flux par connexion
upstream et y fait passer **le même TLS qu'aujourd'hui**, de bout en bout
jusqu'à l'apiserver. L'agent ne voit que des octets chiffrés, ne détient aucun
droit sur son cluster et ne joint qu'une seule destination, fixée de son côté.

### Before / after

```yaml
# aujourd'hui : le proxy doit pouvoir joindre l'apiserver
spec:
  service:
    ExternalService: { url: "https://prod.example.com:6443" }   # exposé
  cert: { ConfigMap: { name: prod-ca, key: ca.crt } }

# avec cette RFC : c'est l'agent qui appelle le proxy
spec:
  service:
    Agent:
      server_name: kubernetes.default.svc     # SAN vérifié (défaut)
      epoch: 0                                # incrémenté = tunnels coupés
      bootstrap:
        ServiceAccountToken:
          subject: system:serviceaccount:proxyauth-agent:proxyauth-agent
          jwks_url: https://oidc.eks.eu-west-1.amazonaws.com/id/ABC/keys
  cert: { ConfigMap: { name: prod-ca, key: ca.crt } }          # inchangé
```

```mermaid
flowchart LR
    subgraph T["aujourd'hui"]
        K1["kubectl"] --> P1["proxy"] -- "TCP + TLS entrant" --> A1["apiserver<br/>(doit être exposé)"]
    end
    subgraph W["avec cette RFC"]
        K2["kubectl"] --> P2["proxy"]
        G["agent<br/>(cluster distant)"] -- "WebSocket sortant<br/>(CONNECT / SOCKS5 possibles)" --> P2
        P2 -. "flux yamux : TLS proxy↔apiserver" .-> G
        G -- "octets chiffrés" --> A2["apiserver<br/>(aucun port entrant)"]
    end
```

Révisé par les décisions 25 et 26 : l'agent appelle une **passerelle
d'agents** (un Deployment à part, ou le pod du proxy en mode `embedded`) qui
tient les tunnels, et le proxy joint la passerelle en `CONNECT`, pod à pod,
dans le cluster relais ; le TLS proxy↔apiserver traverse les deux.

La flèche nouvellement fiable est la connexion de l'agent vers le proxy :
c'est elle qu'authentifie le mTLS de l'agent. Le flux en pointillé ne crée
aucune confiance nouvelle : le proxy vérifie l'apiserver avec `spec.cert`,
comme aujourd'hui.

---

## 2. Motivation

1. **Un apiserver injoignable n'est pas proxifiable.** `connect_upgrade_stream`
   et `build_upstream_client` (`upstream.rs`) supposent une route du proxy
   vers l'apiserver. Un cluster on-prem derrière un NAT, un k3s sur un site
   edge ou un cluster managé à endpoint privé n'en ont pas : aujourd'hui, la
   seule réponse est d'exposer l'apiserver.
2. **Exposer l'apiserver élargit la surface du cluster.** L'ouvrir à Internet,
   même filtré par IP, met l'endpoint le plus sensible du cluster face à des
   attaquants qui ne passent pas par le proxy : sa politique, son audit et son
   fail2login ne protègent alors que les clients honnêtes.
3. **Les réseaux visés n'autorisent qu'une sortie contrôlée.** Un réseau
   d'entreprise ne sort que par un proxy HTTP (`CONNECT`) ou SOCKS5 ; un
   apiserver isolé ne voit Internet qu'à travers un overlay WireGuard /
   Netbird. Le besoin vient d'un déploiement réel du mainteneur, qui a les
   deux types de proxy sortant.
4. **Un VPN site-à-site par cluster ne passe pas à l'échelle.** Chaque cluster
   ajouté coûte une configuration réseau des deux côtés ; un agent installé
   par un chart Helm, en GitOps, ne coûte qu'un `ProxyKubeApi`.

---

## 3. Goals / non-goals

**Goals**

- Un cluster dont l'apiserver n'accepte aucune connexion entrante est
  proxifié, avec la même politique, le même audit et les mêmes upgrades (exec,
  attach, port-forward, watch) qu'un cluster joint directement.
- La sortie de l'agent passe par un proxy `CONNECT` ou SOCKS5, ou par un
  overlay (Netbird) sans configuration propre.
- Le chemin d'authentification vers l'apiserver ne change pas : jeton de
  l'utilisateur, `client_cert`, et plus tard l'échange de la RFC 0010.
- L'agent ne voit jamais le trafic en clair et ne détient aucun droit sur son
  cluster.
- L'agent s'enrôle sans geste manuel en GitOps (§4.1, `ServiceAccountToken`).
- Les tunnels sont visibles dans le statut du `ProxyKubeApi`, dans les
  métriques et dans l'audit.

**Non-goals**

- **Joindre d'autres services du cluster distant** (dashboards, métriques) :
  l'agent deviendrait un pivot réseau générique ; la destination est fixe
  (décision 2).
- **Terminer le TLS dans l'agent, avec impersonation** : l'agent porterait un
  droit d'impersonation et lirait les jetons ; c'est une autre RFC si le besoin
  vient (décision 4).
- **Rejouer un flux coupé** : un `watch` ou un `exec` qui perd son tunnel est
  coupé, le client k8s retente, comme à la perte d'un réplica du proxy
  aujourd'hui (décision 16).
- **Compatibilité avec le konnectivity-agent** : son protocole laisse le
  serveur choisir la destination, ce que la décision 2 interdit (§12).
- **Le proxy derrière Netbird** (ligne « Support Proxy » de la roadmap) : là,
  c'est le proxy qui sort par un overlay, sans agent ; besoin distinct.
- **Un agent pour plusieurs clusters** : un agent sert un `ProxyKubeApi`, son
  identité y est liée.
- **UDP** : l'apiserver ne parle que TCP.
- **L'affichage des tunnels dans le front** : le statut est lisible par
  `kubectl get proxykubeapi` ; une vue dédiée viendra si on la demande.

---

## 4. User-facing design

### 4.1 Configuration

**Côté `ProxyKubeApi`**, la nouvelle variante de `spec.service` :

```yaml
service:
  Agent:
    server_name: kubernetes.default.svc   # défaut ; SAN vérifié sur l'apiserver
    epoch: 0                              # défaut ; voir §4.2
    bootstrap:                            # obligatoire, une seule variante
      ServiceAccountToken:                # GitOps : rien de secret dans git
        subject: system:serviceaccount:proxyauth-agent:proxyauth-agent
        audience: proxyauth               # défaut
        jwks_from:                        # cluster auto-hébergé (décision 24)
          ConfigMap: { name: prod-sa-jwks, key: jwks.json }   # ou Secret
        # ou jwks: '{"keys":[…]}'         # en ligne, figé
        # ou jwks_url: https://…          # émetteur public (EKS, GKE, AKS)
      # OneShotToken: { auto_rotate: false }
      # ExternalCA: { ca_ref: { ConfigMap: { name: agent-ca, key: ca.crt } } }  # phase 4
```

- `spec.cert` et `spec.client_cert` gardent leur sens : la CA de l'apiserver
  distant et le certificat client présenté à l'apiserver. **`spec.cert` est
  obligatoire** avec `Agent` (§4.3) : la CA de l'apiserver ne vient jamais de
  l'agent.
- `OneShotToken` : le contrôleur écrit un Secret `<name>-agent-bootstrap` dans
  le namespace du `ProxyKubeApi`, valable 1 h, à usage unique ; avec
  `auto_rotate: true`, il en écrit un nouveau dès que le précédent est
  consommé. Associé à un `PushSecret` d'External Secrets vers Vault, et à un
  `ExternalSecret` côté distant, il devient GitOps sans code Vault dans le
  proxy.
- `ServiceAccountToken` : l'agent présente un jeton de ServiceAccount
  **projeté** (volume `serviceAccountToken`, `audience: proxyauth`, 10 min),
  jamais le jeton monté par défaut. Le proxy le valide avec le JWKS de l'une
  de trois sources, une seule à la fois (décision 24) : `jwks_from`, un
  ConfigMap ou un Secret relu à chaque enrôlement (cache 30 s), pour un
  cluster dont l'émetteur est interne (kubeadm, k3s) ; `jwks_url`, pour un
  émetteur public (EKS, GKE, AKS, publics même à endpoint privé) ; `jwks` en
  ligne, figé. Le JWKS se lit par `kubectl get --raw /openid/v1/jwks` sur le
  cluster distant ; il est public, aucun secret n'entre dans git. La clé est
  choisie par le `kid` du jeton : un JWKS qui porte l'ancienne et la nouvelle
  clé fait passer une rotation sans coupure.
- `ExternalCA` (phase 4) : pas d'enrôlement ; le certificat de l'agent vient
  d'ailleurs (cert-manager avec un issuer Vault PKI) et le proxy fait confiance
  à `ca_ref`.

**Côté chart du proxy** (`deploy/chart/values.yaml`), tout est désactivé par
défaut. Révisé par les décisions 26 et 27 : les tunnels vivent dans une
**passerelle d'agents**, un Deployment à part (même image, rôle `gateway`),
ou dans le pod du proxy en mode `embedded` pour une petite installation :

```yaml
agentGateway:
  enabled: false
  mode: separate              # separate (Deployment) | embedded (dans le pod du proxy)
  replicaCount: 2
  port: 8444                  # écoute des agents (enrôlement, tunnels)
  connectPort: 8445           # CONNECT du proxy, interne au cluster relais, jamais dans un Ingress
  tlsMode: inner              # inner | passthrough (§5.3)
  ingress: { enabled: false, className: "", host: "", annotations: {} }
  limits:                     # décision 27, toutes réglables
    tunnelsPerAgentIdentity: 4    # par pod de la passerelle, replicas de l'agent confondus
    streamsPerTunnel: 128         # 32 Mio de fenêtre yamux au pire par tunnel
    maxStreamsPerTunnel: 512      # plafond d'une surcharge par cluster
    handshakeTimeoutSeconds: 10
    enrollPerMinute: 10           # par adresse cliente réelle
    enrollPerMinuteTotal: 120     # tous agents confondus
```

Côté `ProxyKubeApi`, un cluster qui en a besoin (un opérateur qui pilote le
cluster enfant à travers le proxy, des clients HTTP/1.1 nombreux) relève sa
borne, sans dépasser le plafond du chart :

```yaml
service:
  Agent:
    limits: { streams_per_tunnel: 384 }   # ≤ maxStreamsPerTunnel, sinon ramené au plafond et averti
```

**Côté chart de l'agent** (`deploy/chart-agent/values.yaml`) :

```yaml
proxy:
  url: wss://agent.proxy.example.com/agent/v1/connect
  cluster: team-a/prod        # namespace/nom du ProxyKubeApi
  caBundle: ""                # CA de l'URL du proxy (PKI privée, Netbird) ; vide = racines système
tlsMode: inner                # doit être celui du proxy
upstream: https://kubernetes.default.svc:443   # seule destination de l'agent
egress:
  proxy:
    url: ""                   # http://… (CONNECT) ou socks5h://… ; vide = variables d'environnement
    credentialsSecret: ""     # clés username / password
bootstrap:
  method: ServiceAccountToken # ou OneShotToken
  tokenSecret: ""             # OneShotToken : Secret qui porte le jeton
identitySecret: proxyauth-agent-identity  # OneShotToken : clé et certificat partagés (§6.5)
replicaCount: 2
```

- Défaut sûr : `agentTunnel.enabled: false`, aucun ingress. Un proxy qui
  n'active pas l'écoute ne présente aucune surface nouvelle.
- `egress.proxy.url` vide : l'agent lit `HTTPS_PROXY`, `ALL_PROXY` et
  `NO_PROXY` ; absentes, il sort en direct. Une valeur vide n'est pas une
  valeur absente pour `caBundle` : vide signifie « racines système ».

### 4.2 Behaviour rules

- **Cluster sans `Agent`** : rien ne change ; l'écoute de l'agent n'est pas
  consultée.
- **Cluster en `Agent` sans tunnel ouvert** : `503` avec le message
  `agent unavailable`, condition `AgentConnected=False`. **Jamais** de repli
  vers une connexion directe : le cluster n'a pas d'URL.
- **Choix du tunnel** : parmi les tunnels de ce réplica pour cette identité,
  celui qui porte le moins de flux ouverts. Un flux par connexion upstream,
  pas par requête : en HTTP/2, toutes les requêtes standard d'un réplica vers
  un cluster partagent **une** connexion, donc un flux (mesuré, §6.6) ; seuls
  les upgrades (exec, attach, port-forward, en HTTP/1.1) ouvrent chacun le
  leur. La répartition entre replicas de l'agent se fait donc par connexion.
- **Le flux ne porte aucune destination.** Ouvrir un flux vers l'agent veut
  dire « une connexion vers ton `upstream` ». Le proxy fait ensuite son TLS
  avec `cached_tls_config` (`kube_redirect/tls.rs`) et
  `ServerName = server_name`.
- ~~**Un tunnel par paire (replica de l'agent, réplica du proxy).**~~
  Révisé par la décision 26 : **chaque replica de l'agent tient un tunnel vers
  un pod de la passerelle**, celui où le load balancer l'envoie (une affinité
  ne gêne pas). Le pod publie la présence dans Redis
  (`proxyk8sauth:agent:<uid>:<pod>:<agent>`, TTL 30 s). Un réplica du proxy
  lit les pods qui tiennent un tunnel pour l'UID et joint l'un d'eux en
  `CONNECT`, pod à pod, par le nom DNS du Service headless de la passerelle
  (`<pod>.<service>.<ns>.svc`), jamais par une adresse lue dans Redis
  (T19). Il en essaie un autre si le premier refuse ; aucun : `503`.
- **Un rolling update du proxy ne coupe aucun tunnel** : ils vivent dans la
  passerelle. Une requête en cours sur un réplica arrêté est coupée et
  retentée par le client, comme aujourd'hui.
- **Enrôlement** : seulement quand l'agent n'a pas de certificat valide (premier
  démarrage, plus de 24 h hors ligne). Ensuite, renouvellement à 2/3 de la vie
  du certificat, par le canal de contrôle d'un tunnel déjà authentifié.
- **`epoch`** : à chaque connexion et chaque renouvellement, l'epoch du
  certificat est comparé à `spec.service.Agent.epoch`. Plus petit : refusé, et
  les tunnels ouverts avec ce certificat sont fermés dans les 30 s (période de
  rafraîchissement du cache CRD).
- **Suppression du `ProxyKubeApi`** : l'UID disparaît, les tunnels sont fermés,
  les renouvellements refusés. Un `ProxyKubeApi` recréé sous le même nom a un
  autre UID et n'hérite d'aucun agent.

### 4.3 Validation

Rejeté par l'admission (règles CEL, comme `AuthenticationConfiguration`) :

| Condition | Rationale |
| --- | --- |
| `Agent` sans `spec.cert` | la CA de l'apiserver viendrait de l'agent, qui pourrait alors faire l'homme du milieu (T4) |
| `Agent` avec `spec.cert: SystemRoots` | l'apiserver d'un cluster distant a une CA privée ; `SystemRoots` validerait n'importe quel certificat public pour `kubernetes.default.svc` présenté par un agent hostile |
| `ServiceAccountToken` sans aucune de `jwks`, `jwks_url`, `jwks_from`, ou avec plus d'une | une seule source de clés (décision 24) |
| `jwks_url` qui n'est pas `https://` | les clés de confiance ne passent pas en clair |
| `subject` qui ne commence pas par `system:serviceaccount:` | seul un ServiceAccount peut être un agent |
| `server_name` qui n'est pas un nom DNS ou une IP | il devient un `ServerName` rustls |
| `epoch` qui diminue | une baisse réhabiliterait des certificats révoqués (règle de transition CEL) |
| `jwks` > 64 Kio | borne de parsing |

Avertissements, dans `status.warnings` (vu par `kubectl get proxykubeapi` et
le dashboard) :

| Condition | Comportement | Visible dans |
| --- | --- | --- |
| l'empreinte de `kube-root-ca.crt` remontée par l'agent ne figure pas dans `spec.cert` | le trafic passe ; le TLS échouera si la CA est vraiment fausse | `status.warnings`, `status.agent.caFingerprint` |
| tunnels ouverts < tunnels attendus pendant 5 min | le trafic passe sur les tunnels restants | `status.agent`, condition `AgentConnected` reason `Degraded` |
| `OneShotToken` sans `auto_rotate` et aucun agent connecté | l'opérateur doit fournir le jeton | `status.warnings` |
| version de l'agent différente de celle du proxy | le trafic passe tant que le protocole est compatible | `status.agent.version` |

### 4.4 API surface

| Method | Path | Auth | Who may call | Limit |
| --- | --- | --- | --- | --- |
| POST | `/agent/v1/enroll` (port `agentTunnel.port`) | jeton d'enrôlement ou jeton de ServiceAccount, dans le corps | un agent pas encore enrôlé | `enrollPerMinute` par adresse, corps ≤ 16 Kio |
| GET (upgrade WebSocket) | `/agent/v1/connect` (même port) | mTLS : certificat d'agent (§5.3) | un agent enrôlé | `tunnelsPerAgentIdentity`, `handshakeTimeoutSeconds` |
| CONNECT | `kubernetes.default.svc:443` sur `connectPort` (décision 25) | `Proxy-Authorization: Bearer` (jeton du Secret `proxy-auth-k8s-gateway-token`), en-tête `X-Proxyauth-Agent: <uid>` | les réplicas du proxy, et eux seuls (NetworkPolicy) | un flux yamux par `CONNECT`, sous `streamsPerTunnel` |

Les deux routes vivent sur une écoute à part : aucune n'est servie sur le port
de l'API utilisateur, et aucune route utilisateur sur ce port. `/agent/v1/enroll`
est documenté par utoipa (`swagger.json` régénéré) ; aucun client front n'en a
besoin.

---

## 5. Architecture

### 5.1 Une requête à travers le tunnel

```mermaid
sequenceDiagram
    participant K as kubectl
    participant P as proxy (réplica)
    participant G as agent
    participant A as apiserver
    G->>P: WebSocket /agent/v1/connect (mTLS agent)
    P-->>G: hello : réplica, réplicas attendus
    K->>P: GET /clusters/ns/prod/api/v1/pods
    P->>P: portes 1 à 8 (inchangées)
    P->>G: ouvre un flux yamux (sans destination)
    G->>A: TCP vers upstream (fixé par l'agent)
    P->>A: TLS dans le flux (spec.cert, server_name)<br/>Authorization de l'utilisateur
    A-->>P: 200
    P-->>K: 200
```

Le flux s'ouvre après toutes les portes : une requête refusée ne coûte rien à
l'agent. Le TLS est négocié entre le proxy et l'apiserver : l'agent relaie des
enregistrements TLS qu'il ne peut ni lire ni modifier sans être détecté.

**Invariant** : parce que l'ouverture d'un flux ne porte aucune adresse et que
l'agent ne connaît qu'une destination, aucun réplica du proxy, même compromis,
ne peut faire joindre à l'agent autre chose que son apiserver.

### 5.2 Identité et enrôlement

```mermaid
flowchart TD
    S["agent démarre"] --> C{"certificat valide<br/>en mémoire ?"}
    C -- "oui" --> T["/agent/v1/connect<br/>(mTLS)"]
    C -- "non" --> B{"bootstrap"}
    B -- "ServiceAccountToken" --> J["jeton projeté<br/>aud=proxyauth"]
    B -- "OneShotToken" --> O["jeton du Secret"]
    J --> E["POST /agent/v1/enroll<br/>CSR + preuve"]
    O --> E
    E --> V{"preuve valide pour<br/>ns/nom du ProxyKubeApi ?"}
    V -- "non" --> R["401, audit agent_refused"]
    V -- "oui" --> I["certificat 24 h<br/>URI SAN proxyauth://ns/nom/uid<br/>extension epoch"]
    I --> T
```

- La **CA des agents** est générée par le leader du contrôleur dans le Secret
  `proxy-auth-k8s-agent-ca` du namespace du proxy (clé ECDSA P-256, 10 ans),
  lue par tous les réplicas, distincte de toute CA de cluster. La signature
  utilise `rcgen` (déjà dans l'arbre en dépendance de test, §8).
- Le certificat d'agent porte l'URI SAN `proxyauth://<ns>/<nom>/<uid>` et
  l'epoch dans une extension privée. Tous les replicas d'un même agent
  partagent cette identité, mais chacun a sa clé et son certificat.
- `OneShotToken` : seul le SHA-256 du jeton est gardé, dans Redis, avec la TTL
  du jeton ; la consommation est un `GETDEL` : deux réplicas ne peuvent pas
  l'accepter tous les deux.
- `ServiceAccountToken` : `jsonwebtoken` (déjà en dépendance de `api`) vérifie
  la signature avec le JWKS, `aud` = `audience`, `exp`, et `sub` = `subject`.
  La clé est choisie par le `kid` (aucun `kid` : chaque clé du jeu est
  essayée). Le JWKS par URL est mis en cache 10 min, et relu tout de suite
  sur un `kid` inconnu, au plus une fois par minute ; `jwks_from` est relu
  avec un cache de 30 s, comme `OidcConfigCache`. Un JWKS vide ou illisible
  refuse l'enrôlement (reason `jwks`), jamais ne le laisse passer.

### 5.3 Les deux modes TLS de l'écoute

```mermaid
flowchart LR
    subgraph I["tlsMode: inner (défaut)"]
        G1["agent"] -- "TLS externe<br/>(ingress, caBundle ou racines)" --> N1["ingress"]
        N1 -- "WebSocket" --> P1["proxy :8444"]
        G1 -. "mTLS interne dans le WebSocket<br/>(CA des agents)" .-> P1
    end
    subgraph S["tlsMode: passthrough"]
        G2["agent"] -- "mTLS direct<br/>(routage SNI)" --> N2["ingress / LB L4"] --> P2["proxy :8444"]
    end
```

- **`inner`** : l'ingress termine le TLS externe ; l'agent ouvre ensuite, dans
  le WebSocket, un TLS mutuel avec le proxy. Le certificat serveur interne du
  proxy est émis par la CA des agents (SAN fixe
  `agent-tunnel.proxyauth.internal`) ; l'agent apprend cette CA à
  l'enrôlement. Fonctionne derrière n'importe quel ingress.
- **`passthrough`** : l'ingress route sur le SNI sans déchiffrer ; le proxy
  termine lui-même un TLS dont le certificat serveur vient du chart
  (`agentTunnel.tls.secretName`) et exige le certificat client à
  `/agent/v1/connect`. Une couche de TLS en moins, mais l'ingress doit savoir
  faire du passthrough.
- L'enrôlement (`/agent/v1/enroll`) ne passe que par le TLS externe dans les
  deux modes : en `inner`, l'ingress voit la preuve d'enrôlement (§7.4, T9).

**Invariant** : dans les deux modes, l'identité de l'agent est établie par un
TLS qui finit dans le pod du proxy ; aucun en-tête posé par l'ingress n'est lu
comme identité.

### 5.4 Sortie de l'agent

L'agent ouvre son TCP vers le proxy de l'une des trois façons :

1. en direct ;
2. par un proxy HTTP : `CONNECT host:443`, avec `Proxy-Authorization: Basic`
   si `credentialsSecret` est donné, écrit avec le client HTTP/1 de `hyper`
   (déjà dans l'arbre) ;
3. par un proxy SOCKS5 (`socks5h://`, résolution DNS par le proxy), avec
   authentification nom/mot de passe (RFC 1929) si donnée, par `tokio-socks`.

Le TLS externe (et, en `inner`, le mTLS interne) passent ensuite dans ce
tunnel : le proxy sortant ne voit qu'un flux chiffré vers `:443`. Un overlay
WireGuard / Netbird est le cas 1 : la route existe au niveau IP, seul
`caBundle` est à fournir si l'URL du proxy a une PKI privée.

---

## 6. Detailed design

### 6.1 CRD (`libs/server/crd/src/service/`)

- `Service::Agent { server_name, epoch, bootstrap }` et
  `AgentBootstrap::{ServiceAccountToken { subject, audience, jwks, jwks_url,
  jwks_from },
  OneShotToken { auto_rotate }, ExternalCA { ca_ref }}` (`ExternalCA` en phase
  4), avec les règles CEL de §4.3. `jwks_from` est un
  `JwksSource::{ConfigMap, Secret} { name, key, namespace }`, soumis à la
  même politique de namespaces que `spec.cert` (`rbac.referencedNamespaces`
  du chart).
- `url_to_call` renvoie, pour `Agent`, `https://<server_name>` : le reste du
  relais (chemins, en-têtes) n'a pas à savoir qu'un tunnel existe.
- `status.agent { tunnels, expected, lastSeen, version, caFingerprint }` et la
  condition `AgentConnected` (reasons `Connected`, `Degraded`, `NoTunnel`,
  `Refused`), au format des conditions de la RFC 0011.

### 6.2 Protocole (`libs/agent/proto/`, nouveau)

- Transport : WebSocket binaire (`tokio-tungstenite`), adapté en
  `AsyncRead + AsyncWrite` (adaptateur maison, quelques dizaines de lignes),
  puis `yamux` (`Mode::Server` côté agent, `Mode::Client` côté proxy : c'est le
  proxy qui ouvre les flux).
- Le flux 1, ouvert par l'agent à la connexion, est le **canal de contrôle** :
  messages `prost` (dérivés à la main, sans `protoc`), longueur préfixée,
  ≤ 64 Kio : `Hello { replica_id, replicas_expected, protocol }`,
  `AgentInfo { version, ca_fingerprint }`, `RenewRequest { csr }`,
  `RenewResponse { cert }`, `Ping`/`Pong`, `Close { reason }`.
- Tout autre flux est un flux de données : l'agent le relie à `upstream`, sans
  lire un octet de contenu.
- `protocol` est un entier ; un réplica refuse une version qu'il ne connaît pas,
  avec `Close { reason: "protocol" }`.

### 6.3 Passerelle et proxy (`libs/server/api/src/agent/`, nouveau)

Révisé par les décisions 25 et 26 : `listener.rs`, `enroll.rs` et
`registry.rs` sont la passerelle ; le proxy ne garde que le `CONNECT`.

- `listener.rs` : l'écoute `agentGateway.port`, axum avec la feature `ws`,
  `handshakeTimeoutSeconds`, mTLS selon `tlsMode`.
- `enroll.rs` : `POST /agent/v1/enroll`, vérifie la preuve (§5.2), signe le CSR,
  écrit `agent_enrolled` ou `agent_refused`.
- `registry.rs` : `TunnelRegistry`, une table en mémoire par réplica, clé =
  identité (`ns/nom/uid`), valeurs = tunnels yamux avec leur compteur de flux ;
  `open_stream(identity) -> Result<yamux::Stream, AgentUnavailable>`. Il publie
  la présence dans Redis (`proxyk8sauth:agent:<uid>:<réplica>:<agent>`, TTL 30 s)
  et ferme les tunnels dont l'epoch ou l'UID ne correspond plus.
- `connect.rs` (passerelle) : l'écoute `connectPort`, un `CONNECT` par flux :
  jeton comparé en temps constant, `X-Proxyauth-Agent` lu, `registry.open_stream`,
  puis `copy_bidirectional` entre la connexion du proxy et le flux yamux ; un
  `407` sans jeton valide, un `503` sans tunnel pour cet UID.
- ~~`upgrade/connect.rs` : `registry.open_stream` à la place du `TcpStream`.~~
  ~~`upstream.rs` : un client `hyper_util` (décision 22).~~ Révisé par la
  décision 25 : **reqwest passe par la passerelle comme par un proxy
  `CONNECT`.** Le client du cluster, toujours mis en cache par cluster
  (`UPSTREAM_CLIENTS`), reçoit `Proxy::custom`, qui choisit le pod de la
  passerelle pour l'UID (présences Redis, cache local), et `Proxy::headers`,
  qui porte `Proxy-Authorization` et `X-Proxyauth-Agent`. Le TLS vers
  l'apiserver (`spec.cert`, `server_name`) passe ensuite dans le tunnel
  `CONNECT`, comme dans un proxy d'entreprise. Les cinq sites reqwest du relais
  (`standard.rs`, `virtual_redirect` ×2, `list_fallback` ×2) ne changent pas.
  `upgrade/connect.rs` envoie son `CONNECT` lui-même, puis fait son TLS actuel.

### 6.4 Côté contrôleur (`libs/server/controller/`)

- ~~Crée la CA des agents si absente (leader seul).~~ La passerelle la crée
  (décision 26) : chaque pod tente un `create` du Secret, l'apiserver n'en
  laisse passer qu'un, les autres relisent après le `409` ; le proxy ne la lit
  plus (T12).
- `OneShotToken` : écrit `<name>-agent-bootstrap` et le hash dans Redis ;
  régénère après consommation si `auto_rotate`, ou sur l'annotation
  `proxyauth.weebo.si.rs/regenerate-bootstrap`.
- Agrège les présences Redis en `status.agent` et `AgentConnected`.

### 6.5 L'agent (`apps/agent/`, nouveau)

- Un binaire, sa propre image `ghcr.io/batleforc/proxyauthk8s-agent`
  (`deploy/build/agent/Containerfile`, multi-stage, image finale distroless
  statique, non-root) et son chart `deploy/chart-agent/` : Deployment de 2
  replicas, PDB, ServiceAccount **sans** RBAC, `automountServiceAccountToken:
  false`, volume projeté pour `ServiceAccountToken`, système de fichiers en
  lecture seule, NetworkPolicy de sortie (DNS, apiserver, proxy ou proxy
  sortant).
- **`ServiceAccountToken`** : clé et certificat en mémoire seulement ; un pod qui
  redémarre se réenrôle avec un nouveau jeton projeté. Aucun RBAC.
- **`OneShotToken`** : clé et certificat dans le Secret `identitySecret` du
  namespace de l'agent, **créé vide par le chart** ; le ServiceAccount de
  l'agent a `get`, `update` et `patch` sur **ce seul Secret** (`resourceNames`,
  pas de `create` : ce verbe ne se restreint pas par nom). Les replicas
  partagent cette identité : le premier qui trouve le Secret vide consomme le
  jeton et l'écrit, les autres le relisent ; le renouvellement est écrit avec
  le `resourceVersion` lu, le perdant d'un conflit relit (décision 21).
  Un pod redémarré ou un replica ajouté ne consomme donc aucun jeton.
- Métriques sur `:9090/metrics`, sondes `/healthz` (processus) et `/readyz`
  (au moins un tunnel ouvert).
- **`jwksPublisher`** (phase 2, `enabled: false` par défaut) : un CronJob du
  chart de l'agent, sous son **propre** ServiceAccount, lit
  `/openid/v1/jwks` (droit donné à tout ServiceAccount par la ClusterRoleBinding
  `system:service-account-issuer-discovery`, installée par défaut depuis
  Kubernetes 1.21) et l'écrit dans un ConfigMap du cluster distant
  (`get`, `update` sur ce seul ConfigMap, `resourceNames`). Un `PushSecret`
  d'External Secrets le pousse vers Vault, un `ExternalSecret` le recrée côté
  proxy, sous le nom que lit `jwks_from`. L'agent, lui, reste sans RBAC.

### 6.6 POC du 2026-10-23

Un POC jetable (hors dépôt) a monté le chemin complet contre un vrai
apiserver, k3s v1.35.9 sous k3d, avec kubectl 1.35 :

- un vrai WebSocket (`tokio-tungstenite` 0.29), l'adaptateur WebSocket →
  `AsyncRead`/`AsyncWrite` (une cinquantaine de lignes), le mTLS interne
  vérifié par `WebPkiClientVerifier` sur une CA d'agents `rcgen`, l'identité
  lue dans l'URI SAN (`x509-parser`), puis yamux 0.14.1 ;
- côté proxy, un client `hyper-util` dont le connecteur ouvre un flux et y
  négocie le TLS vers l'apiserver (`ServerName = kubernetes.default.svc`) ;
- un « gateway » qui termine le TLS de kubectl et en rouvre un vers l'apiserver
  dans un flux, avec le même ALPN : le transport qu'aura le vrai proxy.

| Vérification | Résultat |
| --- | --- |
| agent signé par une autre CA | refusé (`UnknownIssuer`), aucun tunnel enregistré |
| aucun tunnel | `agent unavailable`, pas de repli |
| `GET /version` en HTTP/2 avec le certificat client admin (`spec.client_cert`) | `HTTP/2.0`, `v1.35.9+k3s1` |
| 20 `list pods` en parallèle, HTTP/2 | 20/20, **0 flux yamux de plus** (multiplexés sur un) |
| `kubectl get nodes`, `auth whoami` avec un bearer token de ServiceAccount relayé tel quel | ALPN `h2` ; `system:serviceaccount:default:poc-user` |
| `kubectl exec`, `exec -i` (stdin), 1 Mio sur stdout | `exec-ok`, `stdin-ok`, 1 048 576 octets |
| `kubectl get cm -w` puis création d'un ConfigMap | évènement reçu |
| `kubectl port-forward` vers CoreDNS `:9153`, trois requêtes | 128 métriques, `200 200 200` |
| `kubectl logs -f` | ligne reçue |

kubectl ouvre ses upgrades (exec, port-forward en WebSocket) **sans ALPN** : le
TLS vers l'apiserver suit et reste en HTTP/1.1, ce que `upgrade.rs` exige
déjà.

Non couvert par le POC, à mesurer en phase 3 : le débit et la latence ajoutés,
la perte d'un tunnel en plein flux, les pings WebSocket derrière un vrai
ingress, plusieurs replicas d'agent, et le passage par le vrai pipeline du
proxy (portes, filtres d'upgrade) au lieu du gateway.

### 6.P Proof against a real client

Suite d'intégration de la RFC 0009 (`tests/integration/tests/rfc0012.rs`),
avec le kubectl épinglé du banc (`mise.toml`). L'agent est déployé par
`deploy/chart-agent` dans un namespace `it-remote` du **même** cluster k3d ; le
`ProxyKubeApi` en `Agent` vise l'apiserver de ce cluster, sa CA venant de
`kube-root-ca.crt`. Le proxy tourne à 2 réplicas. Un tinyproxy et un
microsocks, épinglés par digest dans `.it/images.lock`, servent de proxys
sortants. Les deux modes TLS sont exercés en joignant le Service de l'écoute,
sans ingress.

1. `kubectl get pods` sur le cluster en `Agent` (bootstrap
   `ServiceAccountToken`) — succès ; la ligne d'audit porte `upstream_via:
   agent` ; `status.agent.tunnels` vaut 2 (2 replicas d'agent, un tunnel
   chacun vers un pod de la passerelle, décision 26).
2. `kubectl exec -- echo ok` et `kubectl port-forward` à travers le tunnel —
   `ok` reçu, le port relayé répond ; les politiques des RFC 0003 et 0004
   s'appliquent (un exec interdit est refusé en `403`).
3. `kubectl get pods -w` pendant 2 min, avec un pod créé à la fin — l'évènement
   arrive.
4. **Refus** : agent mis à 0 replica — `kubectl get pods` reçoit `503`
   `agent unavailable` ; `AgentConnected=False` reason `NoTunnel` ; aucun log
   de connexion directe du proxy vers l'apiserver.
5. **Refus** : `epoch` passé de 0 à 1 — les tunnels tombent en moins de 30 s,
   une ligne `agent_refused` reason `epoch` est écrite ; avec un nouveau
   bootstrap, l'agent revient.
6. **Refus** : un jeton de ServiceAccount d'un autre ServiceAccount présenté à
   `/agent/v1/enroll` — `401`, `agent_refused` reason `subject`.
7. **Refus** : un agent enrôlé pour `ProxyKubeApi` A configuré pour servir B —
   la connexion est refusée (`identity`) ; B reste `NoTunnel`.
8. `OneShotToken` : deux agents enrôlés en même temps avec le même jeton — un
   seul obtient un certificat, l'autre reçoit `401` ; avec `auto_rotate`, un
   nouveau Secret apparaît.
9. Sortie par `CONNECT` (`egress.proxy.url: http://tinyproxy…`) — le tunnel
   s'ouvre, le log de tinyproxy montre `CONNECT` vers l'écoute ; tinyproxy à 0
   replica, `AgentConnected=False`.
10. Sortie par SOCKS5 (`socks5h://microsocks…`, avec identifiants) — le tunnel
    s'ouvre ; **refus** avec un mauvais mot de passe : aucun tunnel,
    `AgentConnected=False`.
11. `tlsMode: passthrough` — le point 1 réussit aussi dans ce mode.
12. **Refus** : un client sans certificat d'agent sur `/agent/v1/connect` —
    poignée de main refusée ; et une requête utilisateur sur le port de
    l'écoute — `404`.
13. **Refus puis reprise** (`jwks_from`) : le ConfigMap porte un JWKS sans la
    clé du cluster — l'agent redémarré reçoit `401`, `agent_refused` reason
    `signature` ; le ConfigMap corrigé (ancienne clé fictive et vraie clé),
    l'agent s'enrôle dans les 30 s, sans modification du `ProxyKubeApi`.
14. Le proxy à 2 réplicas, la passerelle à 2 pods : `kubectl rollout restart`
    du proxy pendant un `kubectl get pods -w` — `status.agent.tunnels` reste à
    2, aucune ligne `agent_disconnected` ; le watch coupé avec son réplica est
    rouvert par kubectl et reçoit l'évènement suivant.
15. **Refus** : depuis un pod de test du cluster relais, un `CONNECT` vers le
    `connectPort` de la passerelle sans jeton, puis avec un faux — `407`, et
    aucun flux ouvert chez l'agent (`proxyauth_agent_streams_total` inchangé).

Hors suite, avec un pointeur dans `MATRIX.md` : la variante `OneShotToken` +
External Secrets + Vault, `ExternalCA` et l'agent derrière Netbird sont
vérifiés à la main en suivant la doc ; la traversée d'un vrai ingress aussi.

**Deliberately untouched**, so reviewers do not go looking:

- `forwarded.rs` — les en-têtes envoyés à l'apiserver sont les mêmes, l'agent ne
  les voit pas.
- `kube_redirect.rs`, les portes 1 à 8 — le tunnel remplace le socket, pas la
  politique.
- `upgrade/tunnel.rs` — le relais des upgrades et ses filtres (exec,
  port-forward) travaillent sur un `AsyncRead + AsyncWrite`, qu'il vienne d'un
  socket ou d'un flux yamux.
- `Service::ExternalService`, `KubernetesService` — inchangés.

---

## 7. Security

**Rating:** high — une écoute nouvelle, joignable depuis Internet, et une CA
qui émet des identités ; un défaut d'identité ferait passer le trafic d'un
cluster par une machine hostile.

### 7.1 Threat model

Actifs : le trafic utilisateur vers l'apiserver (jetons compris), la clé de la
CA des agents, les preuves d'enrôlement, la disponibilité des clusters en
`Agent`, l'écoute du proxy. Acteurs : un tiers sur Internet, un agent
compromis ou volé, un locataire d'un autre namespace qui écrit son propre
`ProxyKubeApi`, l'ingress et les proxys sortants (vus comme non fiables pour
la confidentialité), un réplica du proxy compromis.

```mermaid
flowchart LR
    X["Internet<br/>(non fiable)"] -- "B1 : écoute :8444" --> P["passerelle d'agents"]
    G["agent<br/>(fiable pour la disponibilité seulement)"] -- "B1 : enrôlement, tunnel" --> P
    N["ingress, proxy sortant<br/>(non fiables)"] -. "transportent B1" .- G
    Q["proxy (réplicas)"] -- "B5 : CONNECT pod à pod<br/>(jeton, NetworkPolicy)" --> P
    Q -. "B2 : TLS dans le CONNECT puis le flux" .-> A["apiserver"]
    G -- "B3 : TCP fixe" --> A
    P -- "B4 : CA, présences" --> R["Secret CA, Redis"]
    Q -- "B4 : présences (lecture)" --> R
```

Révisé par la décision 26 : la passerelle (`P`) tient l'écoute des agents, la
CA et les tunnels ; le proxy (`Q`) la joint par B5, lit les présences et ne
lit plus la CA. B2 part du proxy : ni la passerelle ni l'agent ne le
terminent.

| # | Threat (STRIDE) | Where | Mitigation | Residual risk |
| --- | --- | --- | --- | --- |
| T1 | Spoofing : un tiers se fait passer pour l'agent de `ns/nom` | B1 | mTLS avec la CA des agents ; URI SAN comparé à la CRD en cache (`registry.rs`) à chaque connexion | aucun sans la clé d'un agent ou de la CA |
| T2 | Elevation : l'agent d'un locataire A sert le cluster d'un locataire B | B1 | l'identité contient `ns/nom/uid` ; la preuve d'enrôlement est vérifiée contre **le** `ProxyKubeApi` demandé | aucun |
| T3 | Info disclosure : l'agent lit ou modifie le trafic | B2 | TLS de bout en bout proxy↔apiserver, CA de `spec.cert` (obligatoire, `SystemRoots` refusé) | l'agent voit volumes et horaires ; accepté (mainteneur) |
| T4 | Spoofing : l'agent compromis présente un faux apiserver | B2 | la CA vient de l'admin, jamais de l'agent ; l'empreinte remontée n'est qu'un avertissement | aucun |
| T5 | Elevation : un réplica compromis fait joindre à l'agent une autre cible du cluster distant | B3 | l'ouverture de flux ne porte aucune adresse ; `upstream` vient des values de l'agent (§5.1) | aucun |
| T6 | Spoofing : rejeu d'un `OneShotToken` | B1 | hash SHA-256 dans Redis, `GETDEL` atomique, TTL 1 h | jeton intercepté avant usage : un agent hostile enrôlé jusqu'au prochain `epoch` (voir T3 : il ne lit rien) |
| T7 | Spoofing : jeton de ServiceAccount d'un autre SA, d'une autre audience, ou expiré | B1 | `sub`, `aud`, `exp` et signature vérifiés (`enroll.rs`) | un attaquant qui peut créer un jeton pour ce SA sur le cluster distant est déjà admin de ce cluster |
| T8 | Elevation : un certificat révoqué reste utilisable | B1 | durée 24 h ; epoch et UID vérifiés à chaque connexion et renouvellement ; tunnels fermés en ≤ 30 s ; `epoch` ne peut pas baisser (CEL) | aucun au-delà de 30 s |
| T9 | Info disclosure : l'ingress ou un proxy sortant capture la preuve d'enrôlement | B1 | la preuve est à usage unique (`OneShotToken`) ou courte et liée à une audience (jeton projeté, 10 min) ; jamais le certificat ni la clé, qui restent dans le mTLS | un ingress hostile pendant la fenêtre d'enrôlement enrôle un agent hostile ; voir T3 ; accepté (mainteneur) |
| T10 | DoS : flot de connexions ou d'enrôlements sur `:8444` | B1 | `handshakeTimeoutSeconds`, `enrollPerMinute` par adresse cliente **réelle** (`X-Forwarded-For` lu avec `TRUSTED_PROXY_COUNT`, sans quoi tous les agents derrière l'ingress partagent une adresse), `enrollPerMinuteTotal`, corps ≤ 16 Kio, aucune lecture Redis avant le mTLS (décision 27) | saturation réseau en amont de la passerelle ; hors périmètre |
| T11 | DoS : un agent légitime ouvre des tunnels ou des flux sans fin | B1 | `tunnelsPerAgentIdentity` (4 par pod), `streamsPerTunnel` (128), fenêtre yamux de 256 Kio par flux : 32 Mio au pire par tunnel, 128 Mio par identité et par pod (décision 27 ; 2 Gio avec les bornes de départ) ; messages de contrôle ≤ 64 Kio | une surcharge par cluster relève la borne jusqu'au plafond du chart ; accepté (mainteneur) |
| T12 | Elevation : vol de la clé de la CA des agents | B4 | Secret lisible par le ServiceAccount de **la passerelle** seul (`resourceNames`) ; le proxy, exposé aux utilisateurs, ne la lit plus (décision 26) ; jamais loggée | un voleur émet des identités d'agent : il ne lit rien (T3), mais ses tunnels attirent du trafic qu'il coupe (disponibilité) ; rotation par nouvelle CA et `epoch` de tous les clusters |
| T13 | Tampering : SSRF par `jwks_url` | B4 | `https` seulement, aucune redirection suivie, 64 Kio, 5 s, réponse lue comme JWKS uniquement | le même que `ExternalService.url` aujourd'hui : un locataire fait émettre un `GET` par le proxy ; accepté (mainteneur) |
| T14 | Repudiation : enrôlement ou tunnel sans trace | P | `agent_enrolled`, `agent_refused`, `agent_connected`, `agent_disconnected` ; `upstream_via` sur chaque requête | aucun |
| T15 | Info disclosure : l'écoute de l'agent sert l'API utilisateur, ou l'inverse | B1 | routeurs axum distincts sur des ports distincts | aucun |
| T18 | Elevation : un pod du cluster relais autre que le proxy se sert du `CONNECT` de la passerelle pour joindre les apiservers enfants | B5 | `Proxy-Authorization` (Secret `proxy-auth-k8s-gateway-token`, lisible par le seul ServiceAccount du proxy, comparé en temps constant) ; NetworkPolicy : `connectPort` n'accepte que les pods du proxy ; `connectPort` jamais dans un Ingress ni un Service exposé | il lui faudrait encore les identifiants de l'apiserver, comme pour un apiserver exposé ; accepté (mainteneur, décision 26) |
| T19 | Tampering : une présence forgée dans Redis envoie le `CONNECT`, et son jeton, à une machine hostile | B4 | la présence ne porte qu'un nom de pod, validé comme label DNS ; le proxy compose `<pod>.<service headless>.<ns>.svc`, jamais une adresse lue dans Redis ; un mauvais pod n'a pas le tunnel et répond `503` ; le TLS de l'apiserver (`spec.cert`) refuse tout autre pair | qui écrit dans Redis rend un cluster indisponible ; il ne reçoit ni le jeton ni le trafic |
| T16 | Spoofing : vol de la clé d'agent dans `identitySecret` (`OneShotToken`) | cluster distant | Secret lisible par le seul ServiceAccount de l'agent et les admins du cluster distant ; `epoch` pour révoquer | un voleur prend la place de l'agent : voir T3, il ne lit rien ; accepté (mainteneur) |
| T17 | Tampering : qui écrit la source de `jwks_from` (le ConfigMap, ou le chemin Vault qui l'alimente) choisit les clés acceptées et enrôle un agent hostile | B4 | même politique de namespaces que `spec.cert` ; le `subject` et l'`audience` restent ceux de la CRD ; JWKS borné à 64 Kio, vide ou illisible = refus ; `agent_enrolled` audité | celui qui écrit ce ConfigMap ou ce chemin Vault enrôle un agent, qui ne lit rien (T3) ; accepté (mainteneur, décision 24) |

### 7.2 Authentication and authorisation

- `/agent/v1/enroll` est joignable sans certificat : la preuve est dans le
  corps, vérifiée avant toute écriture ; un échec renvoie `401` sans dire
  laquelle des vérifications a échoué (le détail va dans l'audit).
- `/agent/v1/connect` exige le mTLS, et l'identité doit correspondre à un
  `ProxyKubeApi` existant, d'UID et d'epoch égaux.
- Par défaut, rien n'est ouvert : `agentTunnel.enabled: false`.
- Un contournement de l'authentification de l'agent donne au plus la place de
  l'agent : relayer un TLS qu'il ne peut pas lire (T3). Il ne donne aucun
  accès à l'apiserver ni à l'API du proxy.
- Le cluster distant ne donne aucun droit à l'agent en `ServiceAccountToken` :
  pas de RBAC, pas de jeton monté par défaut. En `OneShotToken`, un seul droit :
  lire et mettre à jour son propre Secret d'identité (T16).

### 7.3 Input handling and injection

- Entrées non fiables : le corps de `/agent/v1/enroll` (JSON ≤ 16 Kio : CSR
  PEM, preuve), les trames WebSocket, les en-têtes yamux, les messages de
  contrôle (≤ 64 Kio), le JWKS d'une URL. Chaque parseur a une borne ; le CSR
  est parsé par `x509-parser` (déjà dans l'arbre) et seule sa clé publique est
  reprise : le sujet et les extensions demandés sont **ignorés**, le
  certificat est construit par le proxy.
- Nouvelles cibles de fuzz : messages de contrôle et corps d'enrôlement
  (`fuzz/`, RFC 0003).
- `jwks_url` : voir T13.

### 7.4 Secrets and cryptography

- CA des agents : ECDSA P-256, `rcgen` pour émettre, `rustls` pour vérifier ;
  aucune primitive maison. Rotation : nouvelle CA et réenrôlement (documenté).
- Certificats d'agent : 24 h, clé générée dans l'agent, jamais transmise.
- `OneShotToken` : 32 octets aléatoires (`rand::rngs::OsRng`), jamais stocké en
  clair côté proxy (hash dans Redis, clair dans le Secret seulement), comparé
  en temps constant.
- Aucun secret dans un log, une métrique ou une réponse d'erreur ; la preuve
  d'enrôlement est un `SecretString`.

### 7.5 Abuse and resource exhaustion

- Un appelant non authentifié peut au plus : ouvrir une connexion TLS
  (coupée après `handshakeTimeoutSeconds`) ou poster 16 Kio à
  `/agent/v1/enroll` (`enrollPerMinute` par adresse, vérification de
  signature avant toute écriture).
- Un agent authentifié est borné par `tunnelsPerAgentIdentity` et
  `streamsPerTunnel` ; un flux yamux a une fenêtre de 256 Kio (décision 27 :
  32 Mio au pire par tunnel avec les défauts).

### 7.6 Existing defences that still hold

- Les portes 1 à 8 de `redirect`, les filtres d'upgrade (`upgrade/tunnel.rs`,
  RFC 0003 et 0004), le filtrage d'en-têtes de `forwarded.rs`.
- La vérification TLS de l'apiserver par `cached_tls_config` et le
  `client_cert` (`tls.rs`), qui passent tels quels dans le flux.
- Le throttling et fail2login de l'API utilisateur, que l'écoute de l'agent ne
  partage pas.

---

## 8. Supply chain

| Dependency / image / tool | Version | Why this one | Licence | Known CVEs (`task audit`) | Maintenance |
| --- | --- | --- | --- | --- | --- |
| `yamux` | 0.14.1 (2026-09) | multiplexage symétrique avec contrôle de flux par flux ; l'écrire serait l'erreur de Konnectivity (§12) | Apache-2.0 OR MIT | aucune connue, à confirmer par `task audit` en phase 1 | libp2p, 4,6 M téléchargements récents |
| `tokio-tungstenite` | **0.29.0**, celle qu'impose la feature `ws` d'axum 0.8.9 (`axum-0.8.9/Cargo.toml:264`), pas la 0.30.0 : une seule version dans `Cargo.lock` (décision 23) | client WebSocket de l'agent ; côté proxy, la feature `ws` d'axum | MIT | à confirmer en phase 1 | très utilisée (97 M) |
| `tokio-socks` | 0.5.3 (2026-05) | client SOCKS5 avec authentification | MIT | à confirmer en phase 2 | 3,9 M |
| `axum` feature `ws` | 0.8.9, déjà là | upgrade WebSocket côté proxy | MIT | inchangé | inchangé |
| `rcgen` | 0.14.10, passe de dépendance de test à dépendance de `api` | émission des certificats d'agent | MIT OR Apache-2.0 | inchangé | inchangé |
| `x509-parser`, `jsonwebtoken`, `prost`, `hyper`, `rand` | déjà dans `Cargo.lock` | CSR, jetons de ServiceAccount, canal de contrôle, `CONNECT`, jeton ; `hyper-util` (`client-legacy`) n'est plus nécessaire (décision 25) : le proxy de reqwest fait le `CONNECT` | inchangé | inchangé | inchangé |
| image de l'agent | distroless statique, épinglée par digest | image finale sans shell | Apache-2.0 | `trivy` dans `task audit` | updatecli |
| tinyproxy, microsocks (banc seulement) | épinglées dans `.it/images.lock` | proxys sortants de §6.P | GPL-2.0 / MIT, jamais livrées | `trivy` sur le banc | à choisir en phase 3 |

- **Rejetés** : `async-http-proxy` (dernière version en 2022, quelques lignes de
  `hyper` font le `CONNECT`) ; `ws_stream_tungstenite` (suit
  `tokio-tungstenite` avec retard ; l'adaptateur fait quelques dizaines de
  lignes) ; `tonic` (§12).
- **Build and release** : une image et un chart de plus, publiés sur GHCR au
  tag comme les autres ; `deploy/build/back/Containerfile` liste les
  `Cargo.toml` du workspace et gagne les deux crates nouvelles ; SBOM et
  signature suivent le chemin des images existantes.
- **Updates** : updatecli pour l'image de base, `task audit` pour les crates.

---

## 9. Data and privacy

| Data | Personal? | Where stored | Retention | Who can read it | Leaves the instance? |
| --- | --- | --- | --- | --- | --- |
| clé et certificat de la CA des agents | non | Secret du namespace du proxy | jusqu'à rotation | ServiceAccount du proxy | non |
| hash du `OneShotToken` | non | Redis | ≤ 1 h | le proxy | non |
| `OneShotToken` en clair | non (secret d'infrastructure) | Secret `<name>-agent-bootstrap` ; Vault si `PushSecret` | jusqu'à consommation | qui lit ce Secret | vers Vault si l'opérateur le choisit |
| clé et certificat d'agent (`OneShotToken`) | non | Secret `identitySecret` du cluster distant | jusqu'à expiration (24 h, renouvelé) | ServiceAccount de l'agent, admins du cluster distant | non |
| présences de tunnel (réplica, identité, version, epoch) | non | Redis | TTL 30 s | le proxy | non |
| `status.agent` | non | CRD | durée de l'objet | qui lit le `ProxyKubeApi` | non |
| adresse IP de l'agent (vue par le proxy) | non (infrastructure) | audit `agent_connected` | celle des logs | l'opérateur | selon la collecte des logs |
| trafic utilisateur dans le tunnel | oui (jetons, ressources) | jamais stocké | — | chiffré pour l'agent, l'ingress et le proxy sortant | vers l'apiserver, comme aujourd'hui |

- Aucune donnée personnelle nouvelle : l'utilisateur et ses groupes sont déjà
  dans l'audit ; `upstream_via` n'en dit rien de plus.
- Les métriques ne portent ni identité d'agent ni cluster (§10.1).

---

## 10. Operability

### 10.1 Observability

- **Metrics (proxy)** : `proxyauth_agent_tunnels` (gauge),
  `proxyauth_agent_streams_total{result}` (`opened`, `unavailable`,
  `limited`), `proxyauth_agent_enrollments_total{method,result}`,
  `proxyauth_agent_bytes_total{direction}`. Pas de label `cluster`, comme la
  RFC 0010 : le détail est dans `status.agent` et l'audit.
- **Metrics (agent)** : `proxyauth_agent_tunnel_up` par réplica du proxy,
  `proxyauth_agent_reconnects_total{reason}`,
  `proxyauth_agent_cert_expiry_seconds`.
- **Logs and audit trail** : `AuditEvent` gagne `upstream_via` (`direct` |
  `agent`) ; évènements `agent_enrolled`, `agent_refused` (reason `subject`,
  `audience`, `signature`, `jwks`, `replay`, `epoch`, `identity`),
  `agent_connected`, `agent_disconnected`, au schéma de la RFC 0008.
- **Alerts** : aucune livrée ; la doc propose `AgentConnected=False` plus de
  5 min, et `proxyauth_agent_cert_expiry_seconds < 3600`.

### 10.2 Failure modes

| Failure | Detected by | Behaviour (fail open / closed) | Recovery |
| --- | --- | --- | --- |
| aucun tunnel pour un cluster | `registry.open_stream` | **fermé** : `503 agent unavailable`, jamais de connexion directe | l'agent se reconnecte (backoff 1 s à 60 s) |
| tunnel coupé pendant un flux | yamux | le flux est coupé, le client k8s retente | automatique |
| Redis injoignable | erreurs du pool | les tunnels déjà ouverts servent ; enrôlement `OneShotToken` **refusé** (pas de `GETDEL`) ; statut figé | automatique |
| pod de la passerelle arrêté | présence expirée (≤ 30 s) ou `CONNECT` refusé | les agents qu'il portait se reconnectent ailleurs (backoff 1 s à 60 s) ; le proxy essaie un autre pod de la présence, sinon `503` | automatique |
| Redis injoignable, côté proxy | erreurs du pool | le cache local des présences (60 s) sert, puis `503` : jamais de connexion directe | automatique |
| rolling update du proxy | — | aucun tunnel coupé ; les requêtes en cours sur un réplica arrêté sont retentées par le client | automatique |
| JWKS par URL injoignable | délai de 5 s | le cache de 10 min sert ; au-delà, enrôlement **refusé** | automatique |
| source de `jwks_from` absente, vide ou illisible | lecture à l'enrôlement | enrôlement **refusé** (reason `jwks`), Event `Warning` ; les tunnels déjà ouverts servent | corriger le ConfigMap ou le Secret, relu dans les 30 s |
| Secret de la CA absent ou illisible | démarrage de l'écoute | l'écoute refuse toute connexion ; `AgentConnected=False` reason `Refused` | le leader le recrée ; une CA recréée oblige à réenrôler |
| apiserver injoignable depuis l'agent | échec du TCP de l'agent | le flux est fermé : le proxy répond `502` | côté cluster distant |
| proxy sortant down | agent | aucun tunnel → cas 1 | automatique |
| certificat de l'agent expiré (agent > 24 h hors ligne) | agent | réenrôlement : sans erreur en `ServiceAccountToken`, nouveau jeton en `OneShotToken` | selon la méthode |

Toujours fermé : aucun échec de l'agent ne fait passer une requête par un autre
chemin que son tunnel.

### 10.3 Performance and capacity

- Un flux ajoute un aller-retour vers l'agent puis vers l'apiserver ; pour un
  `reqwest` réutilisé, ce coût est payé une fois par connexion du pool. À
  mesurer sur le banc en phase 3 (`kubectl get pods`, médiane avec et sans
  agent).
- Mémoire : une fenêtre yamux de 256 Kio par flux actif au plus ; 128 flux
  font 32 Mio au pire par tunnel (décision 27 ; 512 en faisaient 128 Mio).
- Tunnels : M clusters × A replicas d'agent, quel que soit le nombre de
  réplicas du proxy (décision 26) ; 50 clusters et 2 agents font 100
  WebSockets, répartis sur les pods de la passerelle.
- Un saut de plus, pod à pod dans le cluster relais (moins d'une
  milliseconde), entre le proxy et la passerelle.
- Débit : un `port-forward` passe par trois couches de TLS en `inner` ; à
  mesurer, sans engagement chiffré avant la phase 3.

---

## 11. Other concerns

- **Accessibility** : pas d'UI ; le statut passe par la CRD.
- **Internationalisation** : pas de chaîne front ; la doc est écrite en
  français comme le reste.
- **Compliance** : réduit l'exposition des apiservers (ISO 27001 A.8.20,
  sécurité des réseaux) ; chaque enrôlement et tunnel est audité.
- **Licensing** : inchangé ; les nouvelles crates sont permissives et dans
  `deny.toml`.
- **Documentation** : une page « Mode agent » (installation, trois méthodes de
  bootstrap, External Secrets + Vault, sortie CONNECT / SOCKS5, Netbird, les
  deux modes TLS), la référence CRD, la page sécurité (T3, T9), le chart de
  l'agent ; la roadmap.

---

## 12. Alternatives considered

| Alternative | Why rejected |
| --- | --- |
| gRPC bidirectionnel, à la Konnectivity | seul le client ouvre un stream gRPC : il faudrait multiplexer à la main dans un stream (blocage en tête de file sur un `watch` lent, défaut connu de Konnectivity) ou un aller-retour par dial ; HTTP/2 de bout en bout cassé par les proxys d'entreprise (décision 3) |
| Réutiliser le konnectivity-agent | le serveur choisit l'adresse à joindre (contraire à la décision 2) ; son authentification suppose le cluster du serveur |
| HTTP/2 inversé (le proxy client HTTP/2 sur la connexion de l'agent) | lie le tunnel à HTTP : les upgrades (SPDY, WebSocket) passent mal dans un HTTP/2 inversé ; yamux transporte des octets |
| Un tunnel par agent et un saut entre réplicas via Redis | un saut réseau de plus et un état de routage partagé ; la perte d'un réplica coupe tout l'agent (décision 7) |
| L'agent termine la requête et impersonne | l'agent lit les jetons et porte un droit d'impersonation sur son cluster (décision 4) |
| Jeton d'agent statique longue durée | un secret permanent dans git ou un Secret, sans rotation ; les trois méthodes retenues n'en ont pas (décision 11) |
| CA de l'apiserver remontée par l'agent | un agent compromis ferait l'homme du milieu (T4, décision 14) |
| Exposer l'apiserver avec un filtre IP | voir §2.2 |
| VPN (Netbird) côté proxy pour chaque cluster | reste possible (« Support Proxy ») ; coûte un pair et une configuration réseau par cluster |
| Un tunnel par paire (replica d'agent, réplica du proxy), à la Konnectivity | derrière un load balancer à affinité non configurable, l'agent retombe toujours sur le même réplica et n'atteint jamais les autres : `503` sur tous les réplicas sans tunnel ; chaque rolling update du proxy coupe tous les tunnels (décision 26, remplace la 7) |
| Exiger un load balancer sans affinité | l'opérateur n'a pas toujours la main sur son load balancer (cas réel du mainteneur) |
| Router l'agent vers un réplica choisi par un en-tête, à l'ingress | peu d'ingress savent le faire, et pas ceux du mainteneur |
| Client `hyper-util` dans le proxy (décision 22) | il fallait réécrire les cinq sites reqwest du relais (en-têtes, refus des redirections, timeout par requête, bornes de corps) autour d'un type commun : des contrôles déjà prouvés, réécrits dans le chemin chaud ; le `CONNECT` de reqwest les garde tels quels (décision 25) |
| Une socket Unix par cluster, que reqwest joint par `unix_socket()` | un objet local de plus par cluster et par réplica, et il laisse entier le problème de l'affinité (décision 25) |
| Ne rien faire | les clusters non exposés restent hors du proxy |

---

## 13. Rollout and compatibility

- **Default behaviour** : inchangé ; l'écoute est désactivée et aucun cluster
  n'utilise `Agent`.
- **Migration** : aucune ; passer un cluster de `ExternalService` à `Agent` est
  un changement de `spec.service`, réversible.
- **Operator prerequisites** : un nom DNS et un certificat pour l'écoute (ingress
  ou LB) ; côté distant, un namespace pour l'agent et, en
  `ServiceAccountToken`, son JWKS (`jwks_from` ou en ligne) ou l'URL de son
  émetteur.
- **Rollback** : repasser en `ExternalService` ou désactiver l'écoute ; la CA des
  agents reste dans son Secret et peut être supprimée à la main.
- **Feature flag** : `agentTunnel.enabled` (chart) et la variante par cluster.
- **Compatibilité de version** : le champ `protocol` du hello ; un agent plus
  récent que le proxy est refusé proprement, avec un message lisible.

---

## 14. Test plan

- **Unit** (`libs/server/crd`) : règles CEL de §4.3, dont la transition
  d'`epoch`.
- **Unit** (`libs/agent/proto`) : encodage et bornes des messages de contrôle,
  adaptateur WebSocket.
- **Unit** (`libs/server/api/src/agent/`) : vérification des preuves
  (`sub`, `aud`, `exp`, signature), construction du certificat à partir d'un
  CSR, choix du tunnel le moins chargé, calcul de l'expiration.
- **Unit** (`apps/agent`) : `CONNECT` et SOCKS5 contre des serveurs de test en
  mémoire, avec et sans identifiants ; lecture de `HTTPS_PROXY` / `NO_PROXY`.
- **Integration** (`libs/server/api/tests/agent_tunnel.rs`) : un agent en
  processus contre le proxy en processus et un apiserver wiremock en TLS.
- **Proof** : §6.P (`tests/integration/tests/rfc0012.rs`).
- **Security** (`libs/server/api/tests/agent_tunnel.rs`), un test par ligne :
  - T1 `a_cert_from_another_ca_is_refused`
  - T2 `an_agent_cannot_serve_another_proxykubeapi`
  - T3 `the_agent_only_sees_tls_records` (le flux relayé ne contient pas le
    jeton en clair)
  - T4 `spec_cert_is_required_and_system_roots_refused`
  - T5 `a_stream_open_carries_no_destination`
  - T6 `a_one_shot_token_is_accepted_once`
  - T7 `a_token_for_another_subject_or_audience_is_refused`
  - T8 `an_epoch_bump_closes_tunnels_and_refuses_renewal`
  - T9 `enrollment_never_returns_the_private_key`
  - T10 `the_enroll_route_is_rate_limited_before_any_write`
  - T11 `stream_and_tunnel_limits_hold`
  - T12 `the_ca_key_never_reaches_a_log`
  - T13 `jwks_url_refuses_http_and_redirects`
  - T14 `enrollment_and_tunnels_write_audit_events`
  - T15 `user_routes_are_not_served_on_the_agent_port`
  - T16 `the_agent_rbac_is_limited_to_its_identity_secret` (rendu du chart,
    `helm template` : un seul `resourceNames`, aucun `create`)
  - T17 `a_jwks_from_change_is_picked_up_and_an_empty_one_refuses`
  - T18 `a_connect_without_the_proxy_token_is_refused`
  - T19 `a_presence_names_a_gateway_pod_never_an_address`
- **Unit** (décision 27) : une surcharge par cluster au-delà du plafond est
  ramenée au plafond et avertie ; la limite d'enrôlement lit l'adresse réelle.
- **Unit** (`libs/server/api/src/agent/`, décision 24) : choix de la clé par
  `kid`, relecture d'une URL sur `kid` inconnu au plus une fois par minute.
- **Fuzz** : messages de contrôle, corps d'enrôlement.
- **Audit** : `task audit` vert avec `yamux`, `tokio-tungstenite`,
  `tokio-socks` et l'image de l'agent.
- **Existing suites** : `http_contract`, `proxy_security`, `proxy_upgrade`,
  envtest et les `rfcNNNN.rs` du banc inchangés : un cluster sans `Agent` se
  comporte comme avant.

---

## 15. Decisions and open questions

### Resolved

Tranchées au `/grill-me` du 2026-10-23.

| # | Question | Decision |
| --- | --- | --- |
| 1 | Pourquoi un agent | **La connectivité sortante seule** (NAT, réseau privé, edge) ; ne plus stocker de secret côté proxy n'est pas un but. |
| 2 | Périmètre du tunnel | **L'apiserver seul**, destination fixée côté agent : pas de pivot réseau. |
| 3 | Transport | **WebSocket + yamux.** Le proxy ouvre les flux (yamux est symétrique, gRPC non) ; WebSocket traverse ingress et proxys d'entreprise. Contrôle en protobuf au-dessus. |
| 4 | Qui s'authentifie à l'apiserver | **Rien ne change** : jeton de l'utilisateur et `client_cert` dans un TLS de bout en bout ; l'agent est un tuyau opaque. |
| 5 | Authentification de l'agent | **Enrôlement, puis mTLS** avec un certificat court lié à un seul `ProxyKubeApi`. |
| 6 | Livrable de la session | **La RFC seule**, en Draft, avec ses index ; pas de code. |
| 7 | Plusieurs réplicas du proxy | ~~**Un tunnel par paire (replica d'agent, réplica du proxy)**, comme Konnectivity ; pas de saut entre réplicas ni de routage dans Redis.~~ Remplacée par la décision 26. |
| 8 | Déclaration dans la CRD | **`spec.service: Agent`** ; un binaire `apps/agent`, sa propre image et son chart. |
| 9 | CA des agents | **CA dédiée** générée par le contrôleur, certificats de 24 h, URI SAN `proxyauth://ns/nom/uid`, renouvellement à 2/3 par le canal de contrôle. |
| 10 | Révocation | **Pas de CRL** : durée courte, contrôle à chaque connexion et renouvellement, champ `epoch`. |
| 11 | Bootstrap compatible GitOps | **Un enum de méthodes** : `OneShotToken { auto_rotate }` (avec External Secrets + Vault pour la GitOps), `ServiceAccountToken` (défaut documenté, rien de secret), `ExternalCA` en phase 4. Le jeton d'1 h seul rendait la GitOps impossible (objection du mainteneur). |
| 12 | Authentification à travers un ingress | **mTLS dans le WebSocket (`inner`) par défaut**, `passthrough` en option ; `tlsMode` réglé par l'opérateur selon son hébergeur. |
| 13 | Écoute côté proxy | **Port, Service et ingress séparés**, désactivés par défaut, avec leurs limites. |
| 14 | Vérification de l'apiserver | **`spec.cert` fourni par l'admin, jamais par l'agent** ; l'agent remonte une empreinte, pour avertir seulement. |
| 15 | Privilèges de l'agent | **Aucun RBAC**, pas de jeton monté par défaut, non-root, lecture seule, NetworkPolicy de sortie ; destination réglée dans ses values. |
| 16 | HA de l'agent | **2 replicas et une PDB** ; le tunnel le moins chargé ; un flux coupé est retenté par le client. |
| 17 | Statut et observabilité | **Présences dans Redis agrégées par le leader** en `AgentConnected` et `status.agent` ; métriques sans label de cluster ; `upstream_via` dans l'audit. |
| 18 | Release | ~~**0.5.0, avant la v1.0.0**, après la 0.4.0~~ **0.4.0, avec la 0011** (révisée le 2026-10-24 par [PLAN.md](PLAN.md) : la 0012 passe avant la 0010, qui prend la 0.5.0) ; avant la v1.0.0 : elle dépend de la 0011 (conditions), pas de la 0010. Le mainteneur en a besoin plus tôt que la v2.0.0 de la roadmap ; la recommandation (après la v1.0.0) a été écartée. |
| 19 | Preuve sans deuxième cluster | **Un seul cluster k3d**, l'agent dans `it-remote` ; preuves par l'audit, l'agent coupé et l'`epoch`. |
| 20 | Sortie par proxy | **`CONNECT` et SOCKS5**, tous deux dans la suite ; Netbird documenté hors suite avec `caBundle`. La recommandation (sans SOCKS5) a été écartée : le mainteneur a les deux. |
| 21 | Où l'agent garde son certificat en `OneShotToken` | **Dans un Secret du cluster distant, en lecture-écriture** pour l'agent (`get`, `update`, `patch`, `resourceNames`), créé par le chart et partagé par les replicas ; en mémoire seulement en `ServiceAccountToken`, qui reste sans RBAC. Sans lui, chaque redémarrage et chaque replica consommerait un jeton. Tranchée après la rédaction, le 2026-10-23. |
| 22 | Le client upstream des clusters en `Agent` | Remplacée par la décision 25. ~~**`hyper-util`** (`client::legacy::Client` et un connecteur yamux + `tokio-rustls`), pas reqwest : `Conn` est scellé, `connector_layer` ne peut pas l'imiter. Deux POC le prouvent (2026-10-23, §6.6) : en mémoire (pool, flux brut, mauvais `server_name` refusé, jamais le jeton en clair chez l'agent), puis contre k3s 1.35.9 (HTTP/2, certificat client, exec, port-forward, watch, logs, mTLS de l'agent).~~ |
| 23 | Version de `tokio-tungstenite` | **0.29.0**, celle de la feature `ws` d'axum 0.8.9. |
| 24 | JWKS d'un émetteur interne, rotation, GitOps | **Une troisième source, `jwks_from`** (ConfigMap ou Secret, cache 30 s), à côté de `jwks` et `jwks_url` ; clé choisie par `kid`, URL relue sur `kid` inconnu ; CronJob `jwksPublisher` optionnel dans le chart de l'agent, sous son propre ServiceAccount, pour la chaîne External Secrets + Vault. Rejeté : le JWKS fourni par l'agent lui-même (il s'auto-attesterait, contraire au principe de T4). T17 ajouté. Tranchée le 2026-10-24, après l'acceptation : un émetteur interne (kubeadm, k3s, le cas de ce mode) n'avait que `jwks` en ligne, cassé par la moindre rotation. |
| 25 | Comment le proxy fait passer ses requêtes dans un tunnel | **reqwest par un proxy `CONNECT`** : la passerelle (décision 26) est un proxy `CONNECT` authentifié ; `Proxy::custom` choisit le pod pour l'UID, `Proxy::headers` porte `Proxy-Authorization` et `X-Proxyauth-Agent` (reqwest 0.13.5) ; `upgrade/connect.rs` envoie son `CONNECT` lui-même. Le TLS vers l'apiserver passe dans le tunnel `CONNECT`, inchangé. Rejetés : `hyper-util` (réécriture des cinq sites reqwest du chemin chaud et de leurs contrôles), une socket Unix par cluster (objet local de plus, et l'affinité reste entière). Remplace la décision 22. Tranchée le 2026-10-24, pendant la phase 1. |
| 26 | Couvrir tous les réplicas du proxy derrière un load balancer à affinité non configurable | **Une passerelle d'agents** : un Deployment à part (même image, rôle `gateway` ; mode `embedded` dans le pod du proxy pour une petite installation) tient les tunnels, l'enrôlement et la CA ; chaque replica de l'agent tient un tunnel vers le pod où le load balancer l'envoie ; le pod publie la présence dans Redis ; le proxy joint ce pod en `CONNECT`, pod à pod, par le DNS du Service headless. Un rolling update du proxy ne coupe plus les tunnels ; la clé de la CA quitte le proxy (T12). T18 et T19 ajoutés. Rejetés : un load balancer sans affinité exigé (pas toujours configurable), un routage par en-tête à l'ingress (pas d'ingress capable), un relais entre réplicas du proxy (le même saut, sans les deux avantages). Remplace la décision 7. Proposée par le mainteneur le 2026-10-24. |
| 27 | Bornes des tunnels et de l'enrôlement | **Toutes réglables dans le chart**, avec des défauts resserrés : 128 flux par tunnel (32 Mio de fenêtre au pire), 4 tunnels par identité et par pod ; une **surcharge par cluster** (`spec.service.Agent.limits`) bornée par `maxStreamsPerTunnel`, pour un opérateur qui pilote un cluster enfant à travers le proxy ; enrôlement limité par l'adresse cliente réelle (`X-Forwarded-For`, `TRUSTED_PROXY_COUNT`) et par `enrollPerMinuteTotal`. Les bornes de départ laissaient 2 Gio de fenêtre par identité. Tranchée le 2026-10-24. |

### Still open

Aucune.

---

## 16. Implementation phases

| Phase | Content |
| --- | --- |
| 1 | Le chemin complet en `ServiceAccountToken` (les trois sources de JWKS) et `tlsMode: inner` : CRD (`Agent`, CEL, statut, `limits`), `libs/agent/proto`, `libs/agent/pki`, **la passerelle** (rôle `gateway`, modes `separate` et `embedded` : écoute, enrôlement, CA, registre, présences, `CONNECT` authentifié), le proxy en `CONNECT` (`Proxy::custom` dans `upstream.rs`, `upgrade/connect.rs`), `epoch`, limites (décision 27), audit, métriques, binaire, image et chart de l'agent, tests unitaires, d'intégration et de sécurité (T1 à T15, T17 à T19), cibles de fuzz. Utile seul : un cluster non exposé devient proxifiable. |
| 2 | `OneShotToken` (avec `auto_rotate` et le Secret d'identité de la décision 21, T16), `tlsMode: passthrough`, sortie `CONNECT` et SOCKS5, CronJob `jwksPublisher` (décision 24). |
| 3 | Banc : agent dans `it-remote`, tinyproxy et microsocks, `rfc0012.rs`, lignes de `MATRIX.md`, retrait de `PENDING` ; mesures de §10.3 ; documentation. |
| 4 | `ExternalCA` (cert-manager, Vault PKI), hors suite. |

Les contrôles arrivent avec leur surface : la phase 1 n'ouvre pas l'écoute sans
mTLS, limites, `epoch` et audit, et la phase 2 n'ajoute pas `OneShotToken` sans
`GETDEL` ni T6.

---

## 17. Implementation notes

Suivi d'avancement, tenu pendant l'implémentation (branche
`feat/rfc-0012-agent-tunnel`), et révision contre l'arbre à la livraison de
la phase 1 (2026-10-24). Les sections 1 à 16 restent le texte accepté ; les
écarts sont notés ici, et ce que la preuve a observé est dans « État ».

### État

| Étape (phase 1) | État | Notes |
| --- | --- | --- |
| 1a CRD : `Service::Agent`, `AgentBootstrap::ServiceAccountToken`, `JwksSource`, CEL, `validate()` | fait | `crd/src/service/agent.rs` ; 7 tests unitaires, envtest `envtest_agent_crd` 7/7 (CEL, `isIP`, transition d'`epoch`) |
| 1a' fermeture des chemins directs | fait | relais `503 agent unavailable` (`agent_tunnel.rs`), sonde `is_reachable` à `false`, `to_kubeconfig` refusé (`ReachedThroughAgent`) ; tests `crd_runtime` 2/2, `agent_tunnel` 1/1 |
| 1b `libs/agent/proto` : adaptateur WebSocket, messages de contrôle, pilote yamux | fait | crate `agent_proto` ; `ws_io` (trames binaires de 16 Kio au plus, une trame texte échoue), `control` (7 messages, bornés à 64 Kio avant lecture), `mux` (un pilote, des poignées) ; 11 tests ; `task audit` vert avec `yamux` 0.14.1 et `tokio-tungstenite` 0.29.0 |
| 1c-i PKI des agents | fait | crate `agent_pki` : `AgentIdentity`, `AgentCa` (P-256, 10 ans), certificat d'agent (24 h) depuis la seule clé publique du CSR, certificat serveur du tunnel (7 jours) ; 7 tests |
| 1c-ii passerelle : réglages, registre, `CONNECT` | fait | `common::config::AgentGatewaySettings` (`PROXYAUTH_ROLE` `proxy`/`gateway`/`embedded`, ports, Service headless, jeton, bornes de la décision 27) ; `api::agent::registry` (tunnel le moins chargé, réservation d'un flux sous le verrou, `close_where`) ; `api::agent::connect` (hyper, `407`/`400`/`503`/`429`, jeton comparé en temps constant) ; 7 tests, dont T18 |
| 1c-iii passerelle : CA | fait | `api::agent::ca::load_or_create` ; envtest `envtest_agent_ca` (quatre pods en course : une seule CA, jamais remplacée) |
| 1c-iv passerelle : écoute des agents (WebSocket, mTLS interne, contrôle) | fait | `api::agent::listener` (`Gateway`, `router`, `still_valid`) : axum `ws` (messages bornés à 16 Kio), `WsIo`, TLS mutuel sous la seule CA des agents (certificat serveur réémis chaque jour), identité du certificat comparée au `ProxyKubeApi` de Redis (UID, `epoch`, service `Agent`), contrôle `AgentInfo` puis `Hello`, `Ping`/`Pong`, `RenewRequest` (`epoch` revérifié), 90 s de silence ferment le tunnel ; tests `agent_tunnel` 5/5 (tunnel ouvert, flux, renouvellement ; `epoch` passé et CA étrangère refusés) |
| 1c-v passerelle : enrôlement `ServiceAccountToken` | fait | `api::agent::enroll` sur le `router` de l'écoute : corps ≤ 16 Kio, limites par adresse (`TRUSTED_PROXY_COUNT`) et totale à la minute (Redis, fermé sans Redis), `ProxyKubeApi` de Redis, JWKS en ligne / `jwks_from` / `jwks_url` (`JwksCache::key_at`), clé par `kid` (`select_key`), `jwt_validator::verify_with_key` (algorithme de la clé, asymétrique seulement, `exp`, `nbf`, `aud`), `sub` ; certificat pour l'identité du CR, jamais celle de la requête ; `401` nu ; tests `agent_tunnel` 7/7 (enrôlement valide ; sujet, audience, expiration, autre clé, jeton illisible, cluster non `Agent` ou absent refusés) |
| 1c-vi passerelle : présences et révocation | fait | `api::agent::presence` : `run` (toutes les `PRESENCE_REFRESH`), `refresh` (le CR relu par `listener::current_epoch` ; présent, même UID, `Agent` : présence publiée ; sinon retirée et tunnels fermés ; `epoch` relevé : tunnels des anciens certificats fermés, un par un ; Redis en panne : rien ne change), `withdraw` à l'arrêt ; le `Close { reason }` d'une fermeture atteint l'agent (le pilote yamux tourne encore 1 s) ; test `agent_tunnel` 8/8 |
| 1c-vii passerelle : rôle | fait | `api::agent::run` (jeton exigé, CA chargée ou créée, `port` et `connect_port` liés, présences ; à l'arrêt `withdraw`) ; `apps/server` : `gateway` seul (ni contrôleur, ni API, ni découverte OIDC), `embedded` à côté du proxy (un échec de la passerelle arrête le pod, un arrêt sur signal est attendu 5 s), `proxy` inchangé ; sondes `/healthz` et `/readyz` (Redis) sur l'écoute des agents ; `api::server::shutdown_signal` extrait ; tests `agent_tunnel` 10/10, envtest `envtest_agent_ca` 2/2 |
| 1c-viii chart de la passerelle | fait | `agentGateway` (valeurs et schéma ; `tlsMode` limité à `inner`) ; `dp.gateway.yaml` (mode `separate`, rôle `gateway`, son propre ServiceAccount, `subdomain` du Service headless, sondes `/healthz` et `/readyz`) ou le back en rôle `embedded` ; `svc.gateway.yaml` (headless `-gateway-pods` pour le `CONNECT`, ClusterIP `-gateway` pour l'ingress des agents) ; `secret.gateway.yaml` (jeton généré, gardé d'une mise à jour à l'autre par `lookup`) ; `ingress.gateway.yaml` (`/agent/v1/` seul) ; `rbac.gateway.yaml` (`create` sur les Secrets, `get` sur la seule CA, `get` des sources `jwks_from`) ; NetworkPolicy (le `connectPort` depuis les seuls pods du back, T18 ; le back vers la passerelle) ; rendu désactivé identique à avant, `helm lint` et `kubeconform -strict` verts dans les deux modes |
| 1c' proxy en `CONNECT` (décision 25) | fait | `common::agent_gateway::GatewayClient` (présences en sorted set Redis à l'heure de Redis, nom de pod validé, `<pod>.<service>:<port>`, cache 5 s, 60 s si Redis tombe) ; `upstream.rs` : `Proxy::custom` + `Proxy::headers`, jamais `None` (une URL `.invalid` à la place, pour qu'aucun repli direct ne soit possible) ; `upgrade/connect.rs` : `CONNECT` envoyé par hyper ; tests de bout en bout `agent_tunnel` (requête et exec à travers passerelle et tunnel jusqu'à un apiserver TLS) et unitaires (T19) |
| 1e' CRD et contrôleur : limites, `status.agent`, joignabilité | fait | `AgentService.limits.streams_per_tunnel` (≥ 1 à l'admission ; `streams_per_tunnel(default, max)` le borne, la passerelle l'applique au mux et au registre) ; chaque pod de la passerelle publie à côté de sa présence un enregistrement (`presence_info_key` : tunnels, versions, empreintes de CA annoncées par `AgentInfo`) ; `State::agent_presences` ; `is_reachable` d'un cluster `Agent` = au moins une présence ; le contrôleur : `Ready` faux en `AgentNoTunnel`, `status.agent { tunnels, gateways, versions, caFingerprints }`, avis `AgentLimitCapped`, reconcile au plus toutes les 60 s ; CRD régénérée ; tests `controller` (évaluation), `agent_tunnel`, envtest `envtest_agent_crd` 8/8 |
| 1e'' `validate_against: Kubernetes` par la passerelle | fait | `kube_redirect::self_subject_review_through_agent` : le `SelfSubjectReview` part sur le client reqwest du cluster sans identité front-proxy (`ca_only_client`), donc par le `CONNECT` ; `UserAuthError::AgentSelfSubjectReview` (401/403 de l'apiserver = faute de l'appelant pour fail2login, tunnel absent = non) ; `to_kubeconfig` reste refusé ; test `agent_tunnel` 11/11 |
| 1e-iii audit, métriques, fuzz | fait | `Event::{AgentEnrolled, AgentRefused, AgentConnected, AgentDisconnected}` (raison, `agent_epoch`, `agent_version`, `client_ip`), `upstream_via` (`direct` / `agent`) sur chaque ligne d'une requête relayée ; `proxyauth_agent_tunnels`, `proxyauth_agent_streams_total{result}`, `proxyauth_agent_enrollments_total{method,result}`, `proxyauth_agent_bytes_total{direction}` (`to_agent`, `from_agent`) ; cibles `fuzz_agent_control` et `fuzz_agent_enroll` (30 s chacune sans plantage) ; tests T14 `agent_tunnel` 13/13 ; `security.mdx` |
| 1d `apps/agent`, image, chart | fait | `apps/agent` (`Config::from_lookup` des `AGENT_*`, `run`, `Health`) : enrôlement par le jeton projeté, `wss://` (`ws://` sur une adresse de bouclage seulement), mTLS interne, yamux `Mode::Server`, flux de contrôle ouvert par `AgentInfo`, `Ping` toutes les 30 s, renouvellement aux 2/3 sans couper les pings, reconnexion 1 à 60 s, un `Close` de révocation fait réenrôler ; `/healthz`, `/readyz` (tunnel ouvert), `/metrics` (`proxyauth_agent_tunnel_up`, `_reconnects_total{reason}`, `_cert_expiry_seconds`) sur `:9090` ; `AgentKey` dans `agent_pki` ; `deploy/build/agent/Containerfile` (musl statique, distroless `nonroot` épinglé par digest, 12,6 Mo, trivy propre) ; `deploy/chart-agent/` (ServiceAccount sans RBAC, `automountServiceAccountToken: false`, volume projeté jeton + `kube-root-ca.crt`, PDB, NetworkPolicy de sortie) ; CI (`build.yaml`, `release-prepare.yml`), `cog.toml`, `lint:helm`, `audit:image` ; test de bout en bout `the_real_agent_enrols_and_carries_a_relayed_request` |
| 1e tests de sécurité T1 à T15, T17 à T19, fuzz | fait | un test par menace, au nom de §14, dans `agent_tunnel.rs` (28/28, T17 sous `--features envtest`) ; T18 est `connect::tests::a_connect_without_the_proxy_token_is_refused` ; T6 et T16 sont de la phase 2. Trois défauts trouvés et corrigés : `Hello` partait avant l'inscription du tunnel (un tunnel au-delà de `tunnelsPerAgentIdentity` était salué puis coupé), le flux de contrôle comptait dans la borne yamux (`streams_per_tunnel: 1` ne laissait aucun flux de données), et le client JWKS suivait les redirections, `jwks_url` n'étant vérifié en `https://` qu'à l'admission |
| 3 (part) banc, `rfc0012.rs`, `MATRIX.md`, documentation, mesures | fait | banc : image de l'agent construite et importée, `agent.k8s.localhost` dans les `hostAliases`, la passerelle à 2 pods avec son Ingress (`.it/values.yaml`) ; fixture `agent_proxied` (un `deploy/chart-agent` par test, le JWKS du banc en ligne) ; `rfc0012.rs` 10/10 (points 1 à 7, 12, 13, 15) et `rfc0012_restart.rs` 1/1 (point 14, seul) sur le banc k3d ; le reste de la suite Rust inchangé (63/64 en parallèle, le 64e, `rfc0007::p3`, passe seul : limite inotify de l'hôte) ; `MATRIX.md` : 11 lignes `suite`, points 8 à 11 `hors suite` (phase 2) ; 0012 retirée de `PENDING` ; `docs/content/docs/agent.mdx`. Le banc a trouvé deux défauts, corrigés : le pod de la passerelle sans nom DNS (EndpointSlice, voir « Écarts ») et un `401` au lieu de `503 agent unavailable` quand `validate_against: Kubernetes` n'a pas de tunnel. Mesures (§10.3, `measure_the_agent_overhead`, banc k3d sous podman) : `kubectl get pods`, médiane de 21, 81 ms en direct, 131 ms par l'agent (un `CONNECT`, un flux et un TLS de plus par processus kubectl) ; stdout d'un exec, 435 Mio/s en direct, 55 Mio/s par l'agent (trois TLS, WebSocket, yamux) |

### Reprise (à lire en premier après un redémarrage)

Tenu à jour à chaque étape ; c'est d'ici que l'on repart, pas de
l'historique de la conversation.

**Phase 2** : branche `feat/rfc-0012-phase-2`, partie de
`feat/rfc-0012-agent-tunnel` (`1ed52fc`), PR à empiler sur #294. Fait : 2a
(sortie `CONNECT` / SOCKS5, `0cd92ea`), 2b (`tlsMode: passthrough`,
`21d249d`), 2c-i (`OneShotToken` côté proxy, `7419aac`), 2c-ii (agent, `5a926d9`), 2d (`jwksPublisher`).
Ensuite : 2e banc (points 8 à 11, T16, `MATRIX.md`), puis la PR.

**Phase 1** : branche `feat/rfc-0012-agent-tunnel`, partie de `main` après la PR #293
(`d8f7341`), poussée ; **PR #294** ouverte
(`feat: agent mode, reach an apiserver that is not exposed (RFC 0012, phase 1)`). Commits signés (`git commit -s -S
-F <fichier>`), le mainteneur a autorisé les commits sur cette branche ; les
messages sont préparés dans `.git/rfc0011/commit-0012-*.txt` (hors suivi
git), validés par `cog verify --file`. Le hook pre-commit lance `task lint`.

**Modules livrés** (où regarder) :

| Chemin | Rôle |
| --- | --- |
| `libs/server/crd/src/service/agent.rs` | `AgentService`, `AgentBootstrap::ServiceAccountToken`, `JwksSource`, CEL, `validate()` |
| `libs/server/crd_runtime/src/lib.rs` | `is_reachable` à `false`, `to_kubeconfig` refusé (`ReachedThroughAgent`) pour `Agent` |
| `libs/agent/proto/src/{ws_io,control,mux}.rs` | WebSocket en flux d'octets, messages de contrôle bornés, pilote yamux (`mux::new`, `MuxHandle`, `Mode`, `Stream` réexportés) |
| `libs/agent/pki/src/lib.rs` | `AgentIdentity` (`proxyauth://ns/nom/uid/epoch`), `AgentCa` (`generate`, `from_pem`, `issue_agent`, `issue_server`), `identity_of` |
| `libs/server/common/src/config.rs` | `Role`, `AgentGatewaySettings` (`PROXYAUTH_ROLE`, `AGENT_GATEWAY_*`, `POD_NAMESPACE`) |
| `libs/server/common/src/state/redis.rs` | `presence_refresh`, `presence_remove`, `presence_members` (sorted set, heure de Redis) |
| `libs/server/common/src/agent_gateway.rs` | `GatewayClient` (sur `State.agent_gateway`), `presence_key`, `is_pod_name`, `PRESENCE_TTL`, `PRESENCE_REFRESH`, `AGENT_HEADER` |
| `libs/server/api/src/agent/registry.rs` | `TunnelRegistry` (`register` → `TunnelGuard`, `open_stream`, `identities`, `close_where`) |
| `libs/server/api/src/agent/connect.rs` | écoute `CONNECT` de la passerelle (`ConnectService`, `serve`) |
| `libs/server/api/src/agent/ca.rs` | `load_or_create` du Secret de la CA (`create`, relecture sur `409`) |
| `libs/server/api/src/agent/listener.rs` | écoute des agents : `Gateway::new`, `router` (`/agent/v1/connect`, `/agent/v1/enroll`), `still_valid`, `current_epoch` |
| `libs/server/api/src/agent/enroll.rs` | enrôlement `ServiceAccountToken` |
| `libs/server/api/src/agent/presence.rs` | présences et révocation : `run`, `refresh`, `withdraw` |
| `libs/server/api/src/agent/mod.rs` | `run` : la passerelle entière jusqu'au signal (`GatewayError`) |
| `apps/agent/src/{lib,main}.rs` | l'agent : `Config`, `Health`, `run` ; sondes et métriques |
| `deploy/build/agent/Containerfile`, `deploy/chart-agent/` | image et chart de l'agent |
| `libs/server/api/src/cluster/redirect/kube_redirect/{upstream.rs,upgrade/connect.rs}` | le proxy en `CONNECT` (`agent_proxy`, `connect_through_gateway`) |
| `libs/server/api/tests/{agent_tunnel,envtest_agent_crd,envtest_agent_ca}.rs` | bout en bout en processus, CEL et CA sur un vrai apiserver |

**État du dépôt au moment de l'arrêt** (2026-10-24, soir) : la phase 1 est
commitée et poussée jusqu'à `843efdd`. La CI du commit `0f6ce9b` était verte,
banc compris (`integration` : 74/74 puis 3/3, `rfc0012` 10/10 et le point 14),
sauf CodeQL : une alerte (#1483, « cleartext logging ») sur un `{ended:?}` de
`agent_tunnel.rs`, faux positif retiré par `843efdd`. La CI de `843efdd`
tournait encore à l'arrêt. La mise à jour de cette « Reprise » est un commit
local, à pousser une fois la CI de `843efdd` finie : un push qui ne touche que
`.rfc/` annule le banc en cours (`concurrency` d'`integration.yaml`). Le banc
k3d local (`proxyauth-it`, port 8443) tournait encore : `task it:down`, ou
`systemctl --user start podman.socket` pour le réutiliser après un redémarrage.

Pour repartir : `gh pr checks 294` (tout vert attendu ; l'alerte CodeQL #1483
doit être fermée après `Analyze (rust)`), `git status`, puis
`git push` du commit de « Reprise » s'il n'est pas sur `origin`.

**Suite, dans l'ordre** (chaque étape : code, tests, §17 « État », commit) :

1. ~~Commiter le proxy en `CONNECT` puis la CA.~~ Fait.
2. ~~**Écoute des agents** (`api/src/agent/listener.rs`).~~ Fait ; reste à la
   brancher dans `main.rs` (étape 5). La route `enroll` s'ajoute au même
   `router`.
3. ~~**Enrôlement** (`api/src/agent/enroll.rs`).~~ Fait. Pas de cache de
   30 s pour `jwks_from` (`shortcut:` dans le code : borné par
   `enroll_per_minute_total`) ; pas de test de la limite de débit (la clé
   Redis est partagée entre les tests).
4. ~~**Présences et révocation** (`api/src/agent/presence.rs`).~~ Fait ;
   `run` et `withdraw` sont à brancher dans `main.rs` (étape 5).
5. ~~**Rôle dans `apps/server/src/main.rs`**.~~ Fait. Les sondes de la
   passerelle sont `/healthz` et `/readyz` sur `agentGateway.port` (l'écoute
   des agents) : le chart les vise là.
6. ~~**Chart** (`deploy/chart/`).~~ Fait. Restent : HPA et PDB de la
   passerelle, la page `helm-chart.mdx` (phase 3, documentation).
7. ~~**CRD et contrôleur**.~~ Fait, avec deux écarts (plus bas) :
   pas de condition `AgentConnected`, pas de `expected` ni de `lastSeen`.
8. ~~**`to_kube_client` par le `CONNECT`**.~~ Fait autrement : pas de
   client kube (kube ne sait pas poser les en-têtes du `CONNECT`), le seul
   appel qu'en fait `auth_against_kubernetes` passe par reqwest.
9. ~~**Audit et métriques**.~~ Fait. Les raisons d'`agent_refused` sont
   celles du code (`subject`, `token`, `jwks`, `unknown key`, `epoch`,
   `uid`, `identity`, `tls`, `rate limit`…) : `token` regroupe audience,
   signature et expiration, que `jwt_validator` ne distingue pas.
10. ~~**L'agent**.~~ Fait pour la phase 1 : sortie directe seulement
    (`CONNECT` et SOCKS5 sont de la phase 2), `ServiceAccountToken` seul.
    Le chart refuse `tlsMode` autre que `inner` et une autre méthode.
11. ~~Tests de sécurité restants.~~ Faits. **La phase 1 est complète**, prouvée
    sur le banc (phase 3 faite pour les points de la phase 1).
12. ~~Ouvrir la PR~~ : #294. Reste : CI verte sur le dernier commit, relecture
    et merge par le mainteneur.
13. **À décider (mainteneur)** : les phases 2 à 4 avant le tag 0.4.0, ou
    après (`PLAN.md`, étapes 16 et 17).
14. **Phase 2**, une PR, qui livre les points 8 à 11 de §6.P (lignes 8 à 11
    de `MATRIX.md` à passer de `hors suite` à `suite`) :
    - `OneShotToken` : `AgentBootstrap::OneShotToken { auto_rotate }`, SHA-256
      du jeton dans Redis, consommation par `GETDEL` (T6) ; côté agent, clé et
      certificat dans le Secret `identitySecret`, `get`/`update`/`patch` sur ce
      seul Secret (T16, rendu du chart vérifié) ; décision 21.
    - `tlsMode: passthrough` : l'écoute termine elle-même le TLS
      (`agentGateway.tls.secretName`), certificat client exigé ; le chart de
      l'agent accepte alors `tlsMode: passthrough` (son `fail` actuel saute).
    - Sortie de l'agent par `CONNECT` (`hyper`, `Proxy-Authorization: Basic`
      depuis `credentialsSecret`) et SOCKS5 (`tokio-socks` 0.5.3, RFC 1929),
      `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY` ; tinyproxy et microsocks
      épinglés dans `.it/images.lock` pour le banc.
    - CronJob `jwksPublisher` du chart de l'agent (décision 24).
15. **Écarts restés ouverts**, à reprendre quand ils gênent : la borne de
    64 Kio d'un JWKS par `jwks_url` (T13, `JwksCache` partagé avec l'OIDC) ;
    la condition `AgentConnected` (un correctif à proposer au kit
    weebo-operator-kit, jamais édité d'ici) ; T12 complet sans
    `rbac.referencedNamespaces` ; les métriques de la passerelle non exposées
    sur le banc (`WEEBO_METRICS_ADDR`) ; HPA et PDB de la passerelle ;
    `expected` et `lastSeen` de `status.agent`.
16. Phase 4 (`ExternalCA`, cert-manager, Vault PKI), hors suite.

**Lancer les tests** :

- Redis : `podman run -d --rm --name proxyauth-test-redis -p 16379:6379
  docker.io/library/redis:7-alpine redis-server --save ''`, puis
  `TEST_REDIS_URL=redis://127.0.0.1:16379 REQUIRE_TEST_REDIS=1`.
- Banc : `task it:up` (podman : `systemctl --user start podman.socket`),
  puis `task it:test`, ou
  `env -u DBUS_SESSION_BUS_ADDRESS keyctl session - cargo nextest run -p integration --profile it --test rfc0012` ;
  `task it:reload IMAGE=server` puis `rollout restart` de la passerelle après
  un changement du serveur. La suite entière demande
  `fs.inotify.max_user_instances=512` et `kernel.keys.maxbytes=2000000` (root).
- envtest : `KUBEBUILDER_ASSETS=$(dirname $(find
  ~/.local/share/kubebuilder-envtest -name kube-apiserver -type f | head -1))`
  et `--features envtest`, `-j 1` pour un fichier lourd.
- Cibler : `cargo nextest run -p api --test agent_tunnel`, `-p agent_proto`,
  `-p agent_pki`, `-p common agent_gateway`.
- Une suite large peut rester bloquée sur le verrou du répertoire de build
  pendant que l'éditeur lance ses `cargo check` : lancer les tests en
  arrière-plan, et cibler.

| Étape (phase 2) | État | Notes |
| --- | --- | --- |
| 2a sortie de l'agent par `CONNECT` et SOCKS5 (§5.4) | fait | `apps/agent` : `Egress` (`AGENT_EGRESS_PROXY_URL`, sinon `HTTPS_PROXY` / `ALL_PROXY` hors `NO_PROXY`, sinon direct ; `http`, `socks5h`, `socks5` ; identifiants `AGENT_EGRESS_PROXY_USERNAME` / `_PASSWORD`, jamais dans un `Debug`), `dial` pour le tunnel, le même proxy pour le client reqwest de l'enrôlement ; chart : `egress.proxy.url`, `egress.proxy.credentialsSecret` ; `agent.mdx` ; tests unitaires 3 (résolution, `CONNECT` et SOCKS5 contre des serveurs en mémoire, avec et sans identifiants). Écart à §8 : ni `tokio-socks` ni `CONNECT` écrit à la main, les connecteurs `Tunnel` et `SocksV5` de `hyper-util` (`client-proxy`, déjà activé par reqwest) font les deux ; aucun crate nouveau dans `Cargo.lock`. `NO_PROXY` sans plages CIDR ni ports (`shortcut:`) |
| 2b `tlsMode: passthrough` (§5.3) | fait | `common::config::AgentTlsMode` (`AGENT_GATEWAY_TLS_MODE`, `AGENT_GATEWAY_TLS_CERT_PATH` / `_KEY_PATH`, défaut `/agent-tls/tls.{crt,key}`) ; `listener::serve_agents` : `axum::serve` en `inner`, sinon TLS de la passerelle (certificat du chart relu chaque jour, certificat client de la CA des agents facultatif à la poignée de main, une autre CA échoue), `ClientCertificate` posé sur chaque requête, hyper HTTP/1.1 avec upgrades ; `connect` refuse en `401` sans certificat, avant l'upgrade (audit `identity`) ; yamux sur le WebSocket ; certificat illisible : la passerelle ne démarre pas. Agent : `AGENT_TLS_MODE` (`passthrough` exige `wss://`), certificat présenté au TLS externe. Charts : `agentGateway.tlsMode: passthrough` + `agentGateway.tls.secretName` (volume `/agent-tls` en `separate` et `embedded`, sondes en HTTPS, Ingress sans `tls`), `tlsMode` du chart de l'agent ; rendu désactivé identique ; `agent.mdx`. Tests : `agent_tunnel::the_real_agent_opens_a_passthrough_tunnel` (29/29), unitaires `common` et `agent` |
| 2c-i `OneShotToken` : CRD, contrôleur, enrôlement (T6) | fait | `AgentBootstrap::OneShotToken(OneShotTokenBootstrap { auto_rotate })` (CRD régénérée, référence de la doc comprise) ; `common::agent_gateway::bootstrap_key` (`proxyk8sauth:agent-bootstrap:<uid>:<sha256>`, `BOOTSTRAP_TTL` 1 h) ; contrôleur `proxy_kube_api::bootstrap` : Secret `<name>-agent-bootstrap` (clé `token`, 32 octets de `ring`, base64url ; propriétaire le CR ; server-side apply), marque Redis posée avant le Secret, remplacé si absent, consommé ou expiré sous `auto_rotate`, ou sur une nouvelle valeur de `proxyauth.weebo.si.rs/regenerate-bootstrap` (la valeur servie est gardée en annotation du Secret, le CR n'est pas écrit) ; avis `AgentBootstrapNeeded` (aucun tunnel et jeton consommé, ou écriture impossible) ; passerelle : preuve `one_shot_token`, la marque prise par `GETDEL` **après** l'émission du certificat (un CSR refusé ne brûle rien), métrique `method` `one_shot_token` (et `unknown` avant la lecture du corps). Chart : `agentGateway.oneShotToken.enabled` (désactivé) donne au contrôleur `create` et `patch` sur les Secrets, par namespace avec `rbac.referencedNamespaces`. Écart à §7.4 : pas de comparaison en temps constant, le jeton est cherché par son empreinte (la clé Redis), rien n'est comparé. Tests : `agent_tunnel::a_one_shot_token_is_accepted_once` (8 enrôlements en course : un seul certificat), envtest `envtest_agent_bootstrap`, unitaires `must_regenerate`, `bootstrap_key` |
| 2c-ii `OneShotToken` : agent, `identitySecret` (décision 21, T16) | fait | `apps/agent/src/store.rs` : `IdentityStore` (`AGENT_IDENTITY_SECRET`, `AGENT_NAMESPACE`, `AGENT_API_URL` ou `KUBERNETES_SERVICE_HOST`, `AGENT_API_TOKEN_PATH` relu à chaque appel, CA `AGENT_KUBE_CA_PATH`, jamais par le proxy sortant), `GET` puis `PUT` du Secret lu (labels et annotations de Helm gardés, `resourceVersion` lu, `409` = un autre replica a écrit) ; `obtain` : identité du Secret si encore valable (un renouvellement dû passe par le tunnel, pas par un jeton), sinon enrôlement par `one_shot_token` puis écriture, et le perdant relit celle du gagnant ; renouvellement partagé s'il est plus récent ; `revoked` vide le Secret ; `AgentKey::from_pem`. Chart de l'agent : `bootstrap.method: OneShotToken`, `bootstrap.tokenSecret` (exigé), `identitySecret` créé vide sans `data` (une mise à jour ne l'efface pas), Role `get` et `update` sur ce seul nom, aucun `create` (écart à §6.5 : pas de `patch`, inutile) ; jeton d'API projeté (audience de l'apiserver) au lieu du jeton d'enrôlement ; rendu `ServiceAccountToken` identique hors commentaires ; `agent.mdx`. Tests : unitaire `store::tests::the_identity_is_shared_and_a_stale_write_loses`, `agent_tunnel::one_shot_replicas_spend_one_token_and_share_the_identity` (deux agents réels, un jeton, deux tunnels ; 31/31), `agent_pki` `an_agent_key_reloads_from_its_pem`. T16 : le rendu (`resourceNames`, pas de `create`) est vérifié par `helm template` ; le test qui le prouve vient avec le banc (2e) |
| 2d `jwksPublisher` (décision 24) | fait | sous-commande `proxyauthk8s-agent publish-jwks` (`apps/agent/src/jwks.rs`) plutôt qu'une image kubectl : la même image distroless, aucune dépendance de plus ; lit `/openid/v1/jwks`, refuse un JWKS sans clé ou de plus de 64 Kio, n'écrit le ConfigMap (clé `jwks.json`, `PUT` de l'objet lu, `resourceVersion` compris) que s'il change ; `store::ApiClient` partagé avec `IdentityStore`. Chart de l'agent : `jwksPublisher.{enabled,schedule,configMap}`, CronJob (`Forbid`, jeton d'API projeté 10 min), son propre ServiceAccount, ConfigMap créé vide, Role `get`/`update` sur ce seul nom ; `kubeconform -strict` vert ; `agent.mdx`. Test unitaire `jwks::tests::the_jwks_is_published_once_and_an_empty_one_never` |

### Preuve par menace (§7, §14)

| # | Test qui la prouve | Tient ? |
| --- | --- | --- |
| T1 | `agent_tunnel::a_cert_from_another_ca_is_refused` ; banc `rfc0012::p12_no_agent_certificate_no_tunnel_and_no_user_route` | oui |
| T2 | `agent_tunnel::an_agent_cannot_serve_another_proxykubeapi` ; banc `rfc0012::p7_an_agent_cannot_serve_another_cluster` | oui |
| T3 | `agent_tunnel::the_agent_only_sees_tls_records` | oui |
| T4 | `agent_tunnel::spec_cert_is_required_and_system_roots_refused` ; envtest `envtest_agent_crd` | oui |
| T5 | `agent_tunnel::a_stream_open_carries_no_destination` | oui |
| T6 | phase 2 (`OneShotToken`) | — |
| T7 | `agent_tunnel::a_token_for_another_subject_or_audience_is_refused` ; banc `rfc0012::p6_another_service_accounts_token_is_refused` | oui |
| T8 | `agent_tunnel::an_epoch_bump_closes_tunnels_and_refuses_renewal`, `presences_follow_the_cr_and_revoke_its_tunnels` ; banc `rfc0012::p5_an_epoch_bump_revokes_and_the_agent_comes_back` | oui |
| T9 | `agent_tunnel::enrollment_never_returns_the_private_key` | oui |
| T10 | `agent_tunnel::the_enroll_route_is_rate_limited_before_any_write` | oui |
| T11 | `agent_tunnel::stream_and_tunnel_limits_hold` | oui |
| T12 | `agent_tunnel::the_ca_key_never_reaches_a_log` ; le RBAC du chart | en partie : voir « Écarts », sans `rbac.referencedNamespaces` le back lit tous les Secrets |
| T13 | `agent_tunnel::jwks_url_refuses_http_and_redirects` | en partie : les redirections sont refusées par la politique du client, non prouvées par un test ; la borne de 64 Kio n'est pas appliquée |
| T14 | `agent_tunnel::enrollment_and_tunnels_write_audit_events`, `a_relayed_request_is_audited_as_upstream_via_agent` ; banc `rfc0012::p1_…` | oui |
| T15 | `agent_tunnel::user_routes_are_not_served_on_the_agent_port` ; banc `rfc0012::p12_…` | oui |
| T16 | phase 2 (`identitySecret`) | — |
| T17 | `agent_tunnel::a_jwks_from_change_is_picked_up_and_an_empty_one_refuses` (envtest) ; banc `rfc0012::p13_a_jwks_from_without_the_key_refuses_then_recovers` | oui |
| T18 | `connect::tests::a_connect_without_the_proxy_token_is_refused` ; banc `rfc0012::p15_a_connect_without_the_proxy_token_is_refused` | oui |
| T19 | `common::agent_gateway::tests::a_route_names_a_gateway_pod_never_an_address`, `agent_tunnel::a_presence_names_a_gateway_pod_never_an_address` | oui, resserrée (EndpointSlice) |

### Chaîne d'approvisionnement livrée (§8)

| Dépendance | Livrée | Note |
| --- | --- | --- |
| `yamux` | 0.14.1 | |
| `tokio-tungstenite`, `tungstenite` | 0.29.0, une seule version | la feature `ws` d'axum, et `handshake` pour l'agent |
| `prost` | 0.14.4 | sans `protoc` |
| `rcgen` | 0.14.10, dépendance de `agent_pki` | |
| `x509-parser`, `ring`, `jsonwebtoken`, `sha1` | 0.18.1, 0.17.14, 11.1.0, 0.10.7, déjà dans l'arbre | `sha1` vient de la feature `ws` |
| `tokio-socks` | non livrée, inutile | la sortie SOCKS5 (et `CONNECT`) passe par `hyper-util` (`client-proxy`), déjà dans l'arbre ; reqwest gagne sa feature `socks`, sans crate (étape 2a) |
| image de l'agent | `gcr.io/distroless/static-debian12:nonroot`, épinglée par digest (index multi-architecture) | 12,6 Mo ; `trivy image` : 0 HIGH/CRITICAL ; mise à jour par Dependabot (`docker`, `/deploy/build/agent`) : le dépôt n'a pas d'updatecli, contrairement à ce que disait §8 |
| tinyproxy, microsocks | non livrées | banc de la phase 2 |

`task audit` (2026-10-24, avec l'agent dans le graphe) : `cargo deny` advisories, bans, licenses et sources ok (workspace, `fuzz/`, `tests/perf/`), `trivy` sans vulnérabilité connue, `pnpm audit` propre.


### Écarts

- **Pas de condition `AgentConnected`** (§6.1, décision 17) : la
  publication du kit (`weebo_kit_runtime::status::publish`) n'écrit que
  `Ready` et une condition par avis, de type égal à la raison et toujours
  `True` ; un second patch qui ajouterait `AgentConnected` serait défait par
  le suivant, et chaque écriture relance un reconcile. L'état du tunnel est
  donc la raison de `Ready` (`AgentNoTunnel`), le détail `status.agent`.
  `Degraded` et `Refused` attendent un type de condition libre dans le kit
  (correctif à proposer au kit, pas ici).
- **`status.agent` sans `expected` ni `lastSeen`** : `AgentInfo` ne porte pas
  le nombre de replicas de l'agent (phase 1 : `replicas_expected` vaut 1), et
  un horodatage changerait le statut à chaque reconcile, qui en relancerait
  un (le contrôleur ne filtre pas sur `generation`). `gateways`, `versions`
  et `caFingerprints` (listes triées) le remplacent.
- **Le pod de la passerelle se trouve par l'EndpointSlice, pas par le DNS**
  (décision 26 : `<pod>.<service headless>.<ns>.svc`) : un pod de Deployment
  n'a pas de nom DNS à lui (seul un pod qui fixe `hostname` en a, comme ceux
  d'un StatefulSet), ce que le banc a montré. Le proxy lit les EndpointSlices
  du Service headless (`list`, Role du chart), garde les membres de présence
  qui y sont des pods prêts, et joint l'adresse que l'API donne pour ce pod.
  T19 tient, et se resserre : un membre forgé qui nomme un autre pod ne mène
  nulle part.
- **`Hello` après l'inscription** : la passerelle inscrit le tunnel dans le
  registre avant d'envoyer `Hello` ; un tunnel refusé (limite par identité)
  reçoit `Close` à la place. `Hello` vaut acceptation.
- **Le flux de contrôle compte dans la borne yamux** : la passerelle donne à
  yamux `streams_per_tunnel + 1`, le registre borne les flux de données à
  `streams_per_tunnel` ; l'agent accepte jusqu'à 1025 flux.
- **T13, redirections** : le client des `jwks_url` ne suit aucune
  redirection (`Policy::none()`, 5 s) ; le test ne prouve que le refus d'un
  `http://` (un serveur HTTPS de test demanderait une CA que ce client ne
  connaît pas). La borne de 64 Kio d'un JWKS récupéré n'est pas appliquée
  (`JwksCache` est partagé avec les émetteurs OIDC).
- **T12 et le RBAC du back** : sans `rbac.referencedNamespaces`, le
  ClusterRole du back garde un `get` sur tous les Secrets, CA des agents
  comprise ; RBAC n'a pas de refus. T12 ne tient pleinement, en mode
  `separate`, qu'avec `rbac.referencedNamespaces` qui exclut le namespace du
  chart. En mode `embedded`, le back est la passerelle et tient la CA.
- **`Agent(AgentService)`, variante newtype** : la forme YAML est la même
  (`service: { Agent: { … } }`) ; une struct nommée porte les règles CEL
  (`x-kubernetes-validations`), ce qu'une variante struct ne permet pas.
- **`AgentBootstrap` n'a que `ServiceAccountToken` en phase 1** :
  `OneShotToken` (phase 2) et `ExternalCA` (phase 4) arrivent avec leur code ;
  une variante ajoutée à un enum de CRD est un changement compatible.
- **Politique de namespaces de `jwks_from`** (décision 24, §6.1) : c'est
  celle de `CertSource`, c'est-à-dire `PROXYAUTH_ALLOW_CROSS_NS_CERT` (une
  référence vers un autre namespace est ramenée à celui de la CR), et le RBAC
  du chart (`rbac.referencedNamespaces`) en plus : `JwksSource` se convertit
  en `CertSource` pour passer par la même lecture.
- **Trois chemins directs que la RFC ne nommait pas** : la sonde du
  contrôleur (`crd_runtime::is_reachable`), le kubeconfig et le client kube
  de `to_kubeconfig` / `to_kube_client`, dont se sert `auth_against_kubernetes`
  (`api/src/model/user.rs`, `validate_against: Kubernetes`), auraient joint
  `https://kubernetes.default.svc`, c'est-à-dire le cluster du proxy. Ils sont
  fermés dès l'étape 1a : la sonde rend `false`, `to_kubeconfig` une erreur, le
  relais un `503`. Depuis l'étape 7, la sonde lit les présences ; depuis
  l'étape 8, `validate_against: Kubernetes` envoie son `SelfSubjectReview`
  par reqwest et le `CONNECT` (pas de `kube::Client` : kube ne pose pas les
  en-têtes du `CONNECT`). `to_kubeconfig` reste refusé.
- **L'`epoch` est dans l'URI SAN**, pas dans une extension privée :
  `proxyauth://<ns>/<nom>/<uid>/<epoch>`. Une extension demande un OID que le
  projet ne possède pas, et rcgen borne chaque composant d'OID à un `u64`, ce
  qui exclut l'arc `2.25.<uuid>`. L'URI est la seule identité du certificat :
  `identity_of` refuse un certificat qui n'en porte aucune ou plusieurs.
- **Numéro de série aléatoire** (`ring`) : celui de rcgen par défaut est le
  hash de la clé publique, répété à chaque renouvellement avec la même clé.
- **Le contrôle commence par `AgentInfo`** (§5.1, §6.2 : `Hello` d'abord) :
  yamux n'annonce un flux qu'avec sa première trame, et l'agent qui attendrait
  `Hello` sur un flux muet bloquerait les deux bouts. L'agent ouvre le flux en
  écrivant `AgentInfo`, la passerelle répond `Hello`. De même, le premier
  octet d'un flux de données vient de la passerelle (le `ClientHello` TLS du
  proxy), jamais d'une attente de l'agent.
- **yamux parle `futures-io`, pas `tokio::io`** : `WsIo` implémente les
  traits de `futures` et s'enchaîne directement sur yamux ; un flux yamux
  passe par `tokio_util::compat` là où tokio-rustls et `copy_bidirectional`
  l'attendent.
- **Un pilote par connexion yamux** (`mux::new`) : yamux n'avance que si
  `poll_next_inbound` est sondé ; un seul futur possède la connexion, sert les
  ouvertures des poignées (`MuxHandle`, clonables) et range les flux entrants
  dans un canal borné (64), au-delà duquel ils sont refermés.
- **Le proxy ne réessaie pas un autre pod dans la même requête** (§4.2 dit
  « il en essaie un autre ») : un pod qui refuse ou ne répond pas fait
  oublier les présences du cluster (`GatewayClient::forget`), la requête
  échoue en `503`, et la suivante relit Redis. Un réessai demanderait de
  rejouer un corps déjà en partie envoyé.
- **`validate_against: Kubernetes` reste fermé pour un cluster en `Agent`** :
  `to_kube_client` construit un client kube depuis un kubeconfig, qui ne sait
  pas porter les en-têtes du `CONNECT`. À reprendre (un client kube sur un
  connecteur `CONNECT`) avant la fin de la phase 1.
- **La cible du `CONNECT` est ignorée** : la ligne `CONNECT <server_name>:443`
  ne choisit rien, l'agent ne connaît que son `upstream` (invariant de §5.1) ;
  le cluster est désigné par `X-Proxyauth-Agent` seul, et un flux est ouvert
  vers l'agent avant le `200`, pour que le refus porte sa raison.
- **La règle « `Agent` exige `spec.cert` »** porte sur la spec entière
  (`cert` est déjà obligatoire) : elle refuse `SystemRoots` et `Insecure`
  quand `service` est `Agent`.
