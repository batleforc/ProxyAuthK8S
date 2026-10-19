
<!-- Eclipse Che Dedicated instruction --->
# Eclipse che - Cloud Development Environment

## Condition

All the rules in the Eclipse Che block apply only if an env var named WORKSPACE_NAME exists with a non-empty value. The block spans from the `Eclipse Che Dedicated instruction` comment down to the closing `/Eclipse Che Dedicated instruction` comment (i.e. everything up to the "Weebo Dev Env" section).

## Golden Rule

**You don't have root access** No standard system package manager (apt, dnf, ...) can be used. If you need a tool, first search for it in the mise registry, then ask the user before installing it.

**Docker/Podman can't be used** No container engine is directly available. Instead, you can read or update the `devfile.yaml` file that describes the user's environment; any container you need can be added there as a sidecar. After a change, the user must restart the workspace from the local devfile. If a sidecar fails to start on the first try, proceed in two steps: make the container run `sleep infinity`, then create a follow-up task dedicated to starting the actual process inside that sidecar.

**If the `code` cli is not available, use `code-oss`** The `code` cli may not always be present; `code-oss` does the job just as well.

**If you need the full name of the pod, use `$HOSTNAME` or the `hostname` command**

**Watch the storage consumption of folders like `target` or `node_modules`** If those folders grow too big, delete them.

## Command

### Mise

```bash
mise search <CLI NAME>      # Search if a cli/program exists in mise and could be added for your usage
mise use <CLI NAME>         # Install a new cli/program
```

### Code

```bash
code-oss                    # Replaces the plain code cli: add a folder to the workspace or run any other code command
```

## Devfile

A devfile is a yaml file describing a cloud development environment; the schema can be found [here](https://github.com/devfile/api/blob/main/schemas/latest/devfile.json).


<!-- /Eclipse Che Dedicated instruction --->

<!-- WeeboDevEnv Dedicated instruction --->

# Weebo Dev Env - Guideline for the Weebo Dev Env

## Taskfile

All the commands are controlled through Taskfile and need the `desc` field filled in so anyone can understand what the command does.

If multiple commands share the same target (like launching tests), split them into a separate taskfile placed in the `.tasks` folder. Do not use `.task` (singular): that directory is Task's internal cache for checksums/fingerprints and should stay gitignored.

## Mise

Mise is the de-facto package manager. You never have root rights, so use mise whenever a new tool is needed.

## Git / Commits

Commits follow the Conventional Commits format, enforced by [Cocogitto](https://github.com/cocogitto/cocogitto) (`cog.toml`): the commit-msg hook runs `cog verify`, and the pre-commit hook runs `task recu`, `task lint`, gitleaks and the whitespace/end-of-line scripts. Write commit messages accordingly (e.g. `feat: ...`, `fix: ...`).

You don't have the right to commit directly has all commit should be signed and signed-off by the user, if you want to provide a commit message, do so by creating a file and providing the command to validate if the commit message is okay for the user. if you do so, always add that you are a co-owner.

## JS workspace

The JS side is a pnpm workspace (`pnpm-workspace.yaml`) of standalone packages: `apps/front`, `libs/front/front-api` (consumed from source) and `docs`. Each package exposes the same script names when it has the use (`dev`, `build`, `typecheck`, `test`, `lint`); Taskfile aggregates them with `pnpm -r --if-present run <script>` (`.tasks/js.yaml`), so a new package needs no Taskfile change. Nothing is hoisted: declare every import in the package's own `package.json`. A new install script must be reviewed into `allowBuilds`. There is no NX: Cargo is the only Rust build graph.

## Skills

Weebo Dev skills exist and can be found [here](https://github.com/batleforc/weebo-skillsagent). The most important one if you work on UI is the `batlehub-design` skill, which is the design system (tokens, typography, Named Rules). RFCs follow the `batlehub-rfc` skill, and plans are challenged with `grill-me`.

## Doc / Gen / Guideline

### Backend

If you create a backend that exposes an api in Rust, use [Utoipa](https://github.com/juhaku/utoipa) with its Scalar crate for the ui, so the swagger doc is written as you write the api.

### Frontend

If you create a frontend that consumes the backend, use the generated swagger spec and generate the client to call it with [HeyApi](https://github.com/hey-api/hey-api) and its axios plugin.

### Global Doc

For every project, maintain a docs folder, either as plain markdown or with a static fumadocs site to generate the doc (ask the user which one they want).

### Version and security

Always create an audit task (invoked with `task audit`). Its goal is to check for any CVE in the dependencies and the Containerfile. Use the tool matching the stack: `cargo audit` (or `cargo deny`) for Rust, `pnpm audit` for JS/TS, and `trivy` (or `grype`) for the Containerfile/image.

Also, as much as possible, use the latest version of a package as long as it doesn't have a CVE.

### Dockerfile / Containerfile

As much as possible, use a multi-staged build in order to reduce the final image size. If you need a Containerfile reference, you can check [this repo](https://github.com/batleforc/batlehub) which has a Containerfile and a Containerfile.hardened.

<!-- /WeeboDevEnv Dedicated instruction --->

<!-- rtk-instructions v2 -->
## RFC proofs (RFC 0009)

Every numbered point of an RFC's `§6.P` needs a row in `tests/integration/MATRIX.md`; `task lint` runs `task it:matrix` and fails otherwise. An RFC ships its tests in the PR of its code: `tests/integration/tests/rfcNNNN.rs` (one `pN_subject` test per point, through the proxy, never the admin kubeconfig for the action under test) or a spec in `apps/front/e2e-it/` (no `page.route`), its personas in `.it/personas.yaml`, and removes itself from `PENDING` in `.it/matrix.py`. Run them with `task it:up` then `task it:test` (see `docs/content/docs/developper-guide/integration-tests.mdx`).

# Command output

Command output here is condensed to save tokens, keeping every signal and
dropping costly noise. Treat it as the complete result: run commands
normally, and batch related commands into one call to avoid extra turns.
Truncated results state their recovery path in their own output. Re-run a
command as `rtk bypass <cmd>` only when its result is unusable: empty when
output was clearly expected, contradicting its exit code, or garbled.
<!-- /rtk-instructions -->

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- When graphify-out/graph.json exists, run `graphify query "<question>"` before your first grep, search or source-file read for a codebase question, and put this rule in every subagent prompt that explores code. Read raw files only for lines the graph or an earlier read already located. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
- A committed graphify-out/graph.json is rewritten whole on every build, so a large diff to it in a pull request is expected. Do not flag it as churn or suggest skipping the graph rebuild or its hooks.
