
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

## Skills

Weebo Dev skills exist and can be found [here](https://github.com/batleforc/weebo-skills). The most important one if you work on UI is the monofolio skill, which is a design system.

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
# RTK (Rust Token Killer) - Token-Optimized Commands

## Golden Rule

**Always prefix commands with `rtk`**. If RTK has a dedicated filter, it uses it. If not, it passes through unchanged. This means RTK is always safe to use.

**Scope**: RTK applies to shell commands (git, cargo, tests, pnpm, gh, kubectl...). For reading and searching files, Claude Code's native tools (Read, Grep, Glob) remain the priority — do not replace them with `rtk read`/`rtk grep`/`rtk ls`; those are only a fallback when a shell pipeline is genuinely needed.

**Important**: Even in command chains with `&&`, use `rtk`:
```bash
# ❌ Wrong
git add . && git commit -m "msg" && git push

# ✅ Correct
rtk git add . && rtk git commit -m "msg" && rtk git push
```

## RTK Commands by Workflow

### Build & Compile (80-90% savings)
```bash
rtk cargo build         # Cargo build output
rtk cargo check         # Cargo check output
rtk cargo clippy        # Clippy warnings grouped by file (80%)
rtk tsc                 # TypeScript errors grouped by file/code (83%)
rtk lint                # ESLint/Biome violations grouped (84%)
rtk prettier --check    # Files needing format only (70%)
rtk next build          # Next.js build with route metrics (87%)
```

### Test (60-99% savings)
```bash
rtk cargo test          # Cargo test failures only (90%)
rtk go test             # Go test failures only (90%)
rtk jest                # Jest failures only (99.5%)
rtk vitest              # Vitest failures only (99.5%)
rtk playwright test     # Playwright failures only (94%)
rtk pytest              # Python test failures only (90%)
rtk rake test           # Ruby test failures only (90%)
rtk rspec               # RSpec test failures only (60%)
rtk test <cmd>          # Generic test wrapper - failures only
```

### Git (59-80% savings)
```bash
rtk git status          # Compact status
rtk git log             # Compact log (works with all git flags)
rtk git diff            # Compact diff (80%)
rtk git show            # Compact show (80%)
rtk git add             # Ultra-compact confirmations (59%)
rtk git commit          # Ultra-compact confirmations (59%)
rtk git push            # Ultra-compact confirmations
rtk git pull            # Ultra-compact confirmations
rtk git branch          # Compact branch list
rtk git fetch           # Compact fetch
rtk git stash           # Compact stash
rtk git worktree        # Compact worktree
```

Note: Git passthrough works for ALL subcommands, even those not explicitly listed.

### GitHub (26-87% savings)
```bash
rtk gh pr view <num>    # Compact PR view (87%)
rtk gh pr checks        # Compact PR checks (79%)
rtk gh run list         # Compact workflow runs (82%)
rtk gh issue list       # Compact issue list (80%)
rtk gh api              # Compact API responses (26%)
```

### JavaScript/TypeScript Tooling (70-90% savings)
```bash
rtk pnpm list           # Compact dependency tree (70%)
rtk pnpm outdated       # Compact outdated packages (80%)
rtk pnpm install        # Compact install output (90%)
rtk npm run <script>    # Compact npm script output
rtk npx <cmd>           # Compact npx command output
rtk prisma              # Prisma without ASCII art (88%)
```

### Files & Search (60-75% savings)
```bash
rtk ls <path>           # Tree format, compact (65%)
rtk read <file>         # Code reading with filtering (60%)
rtk grep <pattern>      # Search grouped by file (75%). Format flags (-c, -l, -L, -o, -Z) run raw.
rtk find <pattern>      # Find grouped by directory (70%)
```

### Analysis & Debug (70-90% savings)
```bash
rtk err <cmd>           # Filter errors only from any command
rtk log <file>          # Deduplicated logs with counts
rtk json <file>         # JSON structure without values
rtk deps                # Dependency overview
rtk env                 # Environment variables compact
rtk summary <cmd>       # Smart summary of command output
rtk diff                # Ultra-compact diffs
```

### Infrastructure (85% savings)
```bash
rtk docker ps           # Compact container list
rtk docker images       # Compact image list
rtk docker logs <c>     # Deduplicated logs
rtk kubectl get         # Compact resource list
rtk kubectl logs        # Deduplicated pod logs
```

### Network (65-70% savings)
```bash
rtk curl <url>          # Compact HTTP responses (70%)
rtk wget <url>          # Compact download output (65%)
```

### Meta Commands
```bash
rtk gain                # View token savings statistics
rtk gain --history      # View command history with savings
rtk discover            # Analyze Claude Code sessions for missed RTK usage
rtk proxy <cmd>         # Run command without filtering (for debugging)
rtk init                # Add RTK instructions to CLAUDE.md
rtk init --global       # Add RTK to ~/.claude/CLAUDE.md
```

## Token Savings Overview

| Category | Commands | Typical Savings |
|----------|----------|-----------------|
| Tests | vitest, playwright, cargo test | 90-99% |
| Build | next, tsc, lint, prettier | 70-87% |
| Git | status, log, diff, add, commit | 59-80% |
| GitHub | gh pr, gh run, gh issue | 26-87% |
| Package Managers | pnpm, npm, npx | 70-90% |
| Files | ls, read, grep, find | 60-75% |
| Infrastructure | docker, kubectl | 85% |
| Network | curl, wget | 65-70% |

Overall average: **60-90% token reduction** on common development operations.
<!-- /rtk-instructions -->

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
