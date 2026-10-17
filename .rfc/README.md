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
| [REVIEW](REVIEW-roadmap-readme.md) | Revue de la roadmap et du README (2026-10-17) | — | — | — |

Chaque document se termine par une section « Still open » (ou « Questions pour le
grill »), qui sert d'ordre du jour pour le `/grill-me`.

## Ordre de livraison

Le plan détaillé, avec les points de synchronisation entre RFC et les releases
proposées, est dans [PLAN.md](PLAN.md).

- **0.2.0** : 0001 (sortie de NX, workspace pnpm), puis 0002 (refonte UI).
- **Sécurité exec** : 0003, puis 0006 (verbes), puis 0007 (phase 1 dès la 0006, migration après la 0.2.0), puis 0004.
- **0005** (kit) : après la 0001 (pnpm) et une release du kit avec la PR de sa phase 0 ; ne bloque pas la 0.2.0.
