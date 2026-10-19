# ProxyAuthK8S docs

The documentation site, published at <https://batleforc.github.io/ProxyAuthK8S>.
It is built on [`@weebo/operator-docs`](https://github.com/batleforc/weebo-operator-kit)
(layout, theme, routes, search, CRD reference), the shared format of the weebo
operators' sites; only the content and a few components are this project's:

- `src/components/proxykubeapi-generator.*`: the ProxyKubeApi manifest builder,
  shown in place of the kit's `CrdForm` (which cannot render serde `oneOf`
  unions);
- `src/components/api-page*` and `scripts/generate-docs.ts`: the API reference
  from `swagger.json`;
- `src/components/mdx/mermaid.tsx` and `lib/remark-variables.mjs`.

```bash
task docs:dev     # local server, at the root
task docs:crds    # CRD reference (content/docs/crds, public/crd-schemas) from deploy/crd/
task docs:api     # API reference (content/docs/api) from swagger.json
task docs:build   # static export into out/ (DOCS_BASE_PATH=/ProxyAuthK8S in CI)
```

The CRD pages and schemas are generated: never edit them by hand, re-run
`task docs:crds` (`task recu` does).
