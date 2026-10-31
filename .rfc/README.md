# RFC

Format : skill `batlehub-rfc` de
[weebo-skillsagent](https://github.com/batleforc/weebo-skillsagent)
(`skills/batlehub-rfc/template.md`, avec les blocs obligatoires §6.P à §10). Chaque
RFC passe au `/grill-me` avant d'être `In review`.

| # | Titre | Issue | Statut | Risque |
| --- | --- | --- | --- | --- |
| [0001](0001-move-out-of-nx.md) | Sortir de NX, workspace pnpm orchestré par Taskfile | #255 | Implemented | medium |
| [0002](0002-ui-batlehub-design-system.md) | Refonte UI sur le design system BatleHub | #230 | Implemented | medium |
| [0003](0003-port-forward-filtering-gaps.md) | Filtrage des ports : fermer les contournements | #265 (déjà livré), nouvelle issue à créer | Implemented | medium |
| [0004](0004-exec-attach-filtering.md) | Filtrage d'exec et d'attach (commande, conteneur, flux) | à créer | Implemented | high |
| [0005](0005-adopt-weebo-operator-kit.md) | Adopter weebo-operator-kit (doc, tâches) | à créer | Implemented | medium |
| [0006](0006-verb-aware-rules.md) | Règles sensibles au verbe Kubernetes | à créer | Implemented | high |
| [0007](0007-migrate-actix-to-axum.md) | Migrer le serveur HTTP d'actix-web vers axum (hyper 1) | à créer | Accepted (recette après la première instance) | high |
| [0008](0008-audit-trail.md) | Trace d'audit : couverture, schéma et sortie garantie | à créer | Implemented | medium |
| [0009](0009-integration-suite.md) | Banc et suite d'intégration sur un vrai cluster | à créer | Implemented | medium |
| [0010](0010-token-exchange.md) | Échange de jeton : une connexion au proxy pour tous les clusters | à créer | Accepted | high |
| [0011](0011-adopt-kit-runtime-crates.md) | Conditions, Events et codes de raison : adopter les crates du kit | à créer | Implemented | medium |
| [0012](0012-agent-tunnel.md) | Mode agent : joindre un apiserver qui n'est pas exposé | à créer | Implemented | high |
| [0013](0013-agent-version-skew.md) | Versions des agents : les voir, les comparer, refuser un protocole inconnu | #304 | Implemented | medium |
| [0014](0014-trusted-authenticator.md) | Le proxy comme authentificateur de confiance (front-proxy, émetteur) | #323 | Implemented | high |
| [0015](0015-deep-tests.md) | Des tests unitaires et en processus plus profonds | #324 | Implemented | medium |

Chaque document se termine par une section « Still open » (ou « Questions pour le
grill »), qui sert d'ordre du jour pour le `/grill-me`.

## Ordre de livraison

Le plan détaillé, avec les points de synchronisation entre RFC, est dans
[PLAN.md](PLAN.md).

- **0.2.0** : toutes les RFC, de 0001 à 0009. Le tag attend la suite
  d'intégration de la 0009 au vert.
- **0009** passe devant tout ce qui reste à coder : 0007 phases 2 à 6, 0008,
  0004 et 0005 phases 2 à 5 livrent chacune leurs lignes de
  `tests/integration/MATRIX.md`.
- **0.4.0** : la 0011 (conditions, Events, rythme du kit) et la 0012 (mode
  agent, avancée de la v2.0.0) ; la 0012 dépend de la 0011, pas de la 0010, et
  passe avant elle (2026-10-24).
- **0.4.1** : la 0013 (versions et protocole des agents), avant la 0010
  (grill du 2026-10-25).
- **0.5.0** : la 0014 (authentificateur de confiance) et la 0015 (tests
  profonds) ; la 0010 passe en 0.6.0 (2026-10-31).
- **0.6.0** : la 0010 (échange de jeton), relue face au mode agent. La 0014
  en est indépendante : sa phase 1 (la garde de version) passe avant toute
  bascule de mode ; la 0010 range sa config sous `upstream_auth`
  (grill et revues du 2026-10-27).
- **v1.0.0** : les bascules de défauts.
