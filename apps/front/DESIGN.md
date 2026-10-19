# ProxyAuthK8S front — design

The front follows the BatleHub design system, **"The Proof Sheet"**: the
`batlehub-design` skill of
[weebo-skillsagent](https://github.com/batleforc/weebo-skillsagent)
(`skills/batlehub-design/DESIGN.md`). Its tokens are copied as is in
`src/design/tokens.css`, its faces in `public/fonts/` (OFL-1.1, `OFL.txt`).
Every Named Rule applies. This file only records what ProxyAuthK8S adds or
changes, as the skill asks ("Anything a project changes is written into that
project's own `DESIGN.md`").

## Cluster status (Resolution as State)

A cluster has four statuses, each on three channels (pattern, word, hue),
rendered by `features/cluster/ClusterStatus.vue`:

| Status      | Pattern                          | Hue         | Resolution state reused  |
| ----------- | -------------------------------- | ----------- | ------------------------ |
| Available   | fine 3×3, all lit                | `--ink`     | `cached`                 |
| Unreachable | coarse 2×2, all lit              | `--accent`  | `blocked`                |
| Unknown     | coarse 2×2, 3 of 4               | `--copper`  | `held` (never "healthy") |
| Disabled    | no pattern, dashed `--rule-soft` | `--ink-dim` | —                        |

The resolve animation is not used: a cluster list does not stream rows in.

## The one action per view

- Home (signed in): **Connect to the cluster**, for the selected row only.
  Rows are selected by a control, the selection is a 1px crimson left edge,
  ink and the raised fill (the bay grammar), flipped in place.
- Cluster callback / no-SSO: **Download the kubeconfig** of the visible tab.
- Landing: **Log in**. CLI: **Copy the workflow**. Error panels: **Back to
  home**.

## Code and kubeconfig

`components/ui/code-block` is written for this front, not ported: a
kubeconfig or a command is set as **ink on `--ground-sunk`**, without syntax
colours. Highlighting would add hues the One Synthetic Rule refuses, and
shiki's HTML would need `v-html` and inline styles the CSP refuses. The block
scrolls inside itself and is focusable.

## Page titles

`PageHeader` `poster` (`--t-display`, 56 → 72px) steps down to 24px
(`--t-px-md`) below 640px: Silkscreen does not hyphenate, and "ProxyAuthK8S"
at 56px overflows a 360px screen. Both are integer ems.

## Notices

`useToast` (the stores' messages, copy confirmations) lands in
`app/Notices.vue`: two live regions always present (errors `role="alert"`,
the rest `role="status"`), framed boxes on `--ground-sunk`, no shadow,
bottom of the viewport.

## Ported primitives

`components/ui/{button,empty-state,page-header,resolution,skeleton,table,tabs}`
and `lib/utils.ts` are copied from `batlehub/ui/src` (Apache-2.0), with
radix-vue moved to reka-ui and the changes above. They are kept out of
Prettier so `task ui:diff-batlehub` shows only real differences.
