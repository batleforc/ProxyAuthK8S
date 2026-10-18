# RFC

Format : skill `batlehub-rfc` de
[weebo-skillsagent](https://github.com/batleforc/weebo-skillsagent)
(`skills/batlehub-rfc/template.md`, avec les blocs obligatoires §6.P à §10). Chaque
RFC passe au `/grill-me` avant d'être `In review`.

| # | Titre | Issue | Statut | Risque |
| --- | --- | --- | --- | --- |
| [0001](0001-move-out-of-nx.md) | Sortir de NX, workspace pnpm orchestré par Taskfile | #255 | In review | medium |
| [0002](0002-ui-batlehub-design-system.md) | Refonte UI sur le design system BatleHub | #230 | In review | medium |
| [0003](0003-port-forward-filtering-gaps.md) | Filtrage des ports : fermer les contournements | #265 (déjà livré), nouvelle issue à créer | In review | medium |
| [0004](0004-exec-attach-filtering.md) | Filtrage d'exec et d'attach (commande, conteneur, flux) | à créer | In review | high |
| [0005](0005-adopt-weebo-operator-kit.md) | Adopter weebo-operator-kit (doc, tâches) | à créer | In review | medium |
| [0006](0006-verb-aware-rules.md) | Règles sensibles au verbe Kubernetes | à créer | In review | high |
| [0007](0007-migrate-actix-to-axum.md) | Migrer le serveur HTTP d'actix-web vers axum (hyper 1) | à créer | In review | high |
| [0008](0008-audit-trail.md) | Trace d'audit : couverture, schéma et sortie garantie | à créer | Accepted | medium |
| [0009](0009-integration-suite.md) | Banc et suite d'intégration sur un vrai cluster | à créer | In review | medium |
| [0010](0010-token-exchange.md) | Échange de jeton : une connexion au proxy pour tous les clusters | à créer | Accepted | high |
| [REVIEW](REVIEW-roadmap-readme.md) | Revue de la roadmap et du README (2026-10-17) | — | — | — |

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
- **v1.0.0** : les bascules de défauts.
