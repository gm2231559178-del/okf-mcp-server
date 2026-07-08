# OKF MCP Server — Development Plan

**Status:** Draft for implementation
**Target spec:** Open Knowledge Format (OKF) v0.1 — https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md
**Implementation language:** Rust

---

## 1. Purpose

An MCP server that lets agents (and humans, via any MCP client) **read, search, and write** OKF knowledge bundles — directories of markdown-with-YAML-frontmatter concept files — backed by either the local filesystem or a git repository, with explicit, auditable control over commit and push.

This is not a new knowledge service and does not reinterpret OKF. It is a thin, spec-conformant read/write/navigate layer over bundles that already live as files.

### 1.1 Goals

- Expose bundle read/list/search/graph operations to any MCP client
- Expose safe, spec-conformant write operations (concept CRUD, index maintenance, log append, citation append)
- Support both plain-filesystem bundles and git-backed bundles, with git commit/push as explicit, separate actions from writing
- Enforce the 3 hard OKF conformance rules on write; treat everything else as advisory
- Support multiple registered bundles per server instance

### 1.2 Non-goals (v1)

- Tier-2 (`tantivy`) full-text index is deferred to Phase 4 as a scale option — tier-1 grep-based full-text search is in scope from Phase 2 (see §5.5)
- Cross-process write coordination (multiple independent server instances writing the same bundle concurrently) — single-process in-memory locking only (see §3.3)
- Auto-merge/conflict resolution on git pull
- A fixed concept-type taxonomy or content schema beyond what the spec requires
- Real-time collaborative editing / locking across multiple concurrent writers
- Rendering (graph visualization is the existing reference visualizer's job, not this server's)

---

## 2. OKF spec recap (source of truth for validation logic)

- **Bundle**: directory tree of `.md` files. May be plain directory, tarball, or git repo.
- **Concept**: one markdown file = one concept. Concept ID = path minus `.md` (e.g. `tables/orders.md` → `tables/orders`).
- **Reserved filenames** (any directory level, MUST NOT be used as concept documents): `index.md`, `log.md`.
- **Frontmatter**: YAML block delimited by `---`. Required field: `type` (non-empty string). Recommended: `title`, `description`, `resource`, `tags`, `timestamp`. Producers may add arbitrary extra keys — consumers must preserve them on round-trip and must not reject unknown keys/types.
- **Body**: free markdown. Conventional (not required) section headings: `# Schema`, `# Examples`, `# Citations`.
- **Links**: two forms —
  - bundle-root-relative, e.g. `/tables/customers.md` (recommended, stable across moves)
  - relative, e.g. `./other.md`
  Links are untyped; relationship kind is conveyed by prose, not link syntax. Broken links are tolerated, not errors.
- **`index.md`**: optional at any directory level. No frontmatter, **except** the bundle-root `index.md`, which is the only file allowed to declare `okf_version: "0.1"` in frontmatter. Body groups entries under `# Heading` sections of `* [Title](path) - description`.
- **`log.md`**: optional at any directory level. Format: `## YYYY-MM-DD` headings (ISO 8601, newest-first), bullet entries, conventionally bold-prefixed (`**Update**`, `**Creation**`, `**Deprecation**` — convention, not enforced).
- **Citations**: `# Citations` section, numbered list `[1] [label](path-or-url)`.
- **Conformance (§9) — exactly 3 hard rules**:
  1. Every non-reserved `.md` file has parseable YAML frontmatter.
  2. Every frontmatter block has a non-empty `type`.
  3. Reserved filenames, when present, follow §6/§7 structure.
  All other constraints (missing optional fields, unknown types, unknown keys, broken links, missing index.md) are advisory only — consumers MUST NOT reject on these.

---

## 3. Architecture

```
┌─────────────────────────────────────────────┐
│                MCP Server                    │
│  ┌─────────────┐  ┌──────────────────────┐  │
│  │  Resources  │  │        Tools          │  │
│  │ okf://...   │  │ okf_read_concept, etc │  │
│  └──────┬──────┘  └──────────┬───────────┘  │
│         └──────────┬─────────┘              │
│              ┌──────▼──────┐                 │
│              │  BundleRepo  │  (per-bundle facade:
│              │              │   parsing, validation,
│              │              │   graph, search)
│              └──────┬──────┘
│         ┌───────────┴────────────┐
│  ┌──────▼──────┐         ┌───────▼───────┐
│  │ LocalFsStore │         │   GitStore    │
│  │  (impl       │         │  (impl        │
│  │  BundleStore)│         │  BundleStore  │
│  │              │         │  + GitControl)│
│  └──────────────┘         └───────────────┘
│         │                          │
│    plain directory            git2 (libgit2) working tree
└─────────────────────────────────────────────┘
```

- **`BundleRepo`**: bundle-level logic that's backend-agnostic — frontmatter parsing/serialization, concept ID resolution, link extraction, graph/backlink computation, search, validation. Operates on top of a `BundleStore`.
- **`BundleStore`**: raw byte-level file operations (list/read/write/delete) for one bundle.
- **`GitControl`**: additional trait implemented only by `GitStore` — status/diff/commit/push/pull/branch. Not exposed for FS-only bundles.
- **Bundle registry**: server config maps a bundle name → `{ backend: fs | git, path, git_config? }`. A single server instance can host multiple bundles.

### 3.1 Data model

```rust
struct ConceptId(String); // e.g. "tables/orders" — normalized, no .md, no leading slash

struct Frontmatter {
    r#type: String,                    // required, non-empty
    title: Option<String>,
    description: Option<String>,
    resource: Option<String>,
    tags: Option<Vec<String>>,
    timestamp: Option<String>,         // RFC3339; server sets/bumps on write unless caller pins it
    #[serde(flatten)]
    extra: serde_yaml::Mapping,        // preserve unknown producer fields verbatim
}

struct Concept {
    id: ConceptId,
    frontmatter: Frontmatter,
    body: String,
}

struct Link {
    source: ConceptId,
    target_raw: String,     // as written: "/tables/x.md" or "./x.md"
    target_resolved: Option<ConceptId>, // None if unresolved (broken link — not an error)
}

struct BundleRoot {
    okf_version: Option<String>, // only meaningful on root index.md
}
```

### 3.2 Traits

```rust
trait BundleStore {
    fn list_files(&self, prefix: Option<&str>) -> Result<Vec<String>>; // raw relative paths
    fn read_raw(&self, path: &str) -> Result<String>;
    fn write_raw(&self, path: &str, content: &str) -> Result<()>; // atomic: temp + rename
    fn delete_raw(&self, path: &str) -> Result<()>;
    fn exists(&self, path: &str) -> bool;
}

trait GitControl {
    fn status(&self) -> Result<GitStatus>;               // staged / unstaged / untracked
    fn diff(&self, path: Option<&str>) -> Result<String>;
    fn commit(&self, message: &str, author: Option<&str>) -> Result<String>; // returns commit sha
    fn push(&self, remote: &str, branch: &str) -> Result<()>;
    fn pull(&self, remote: &str, branch: &str) -> Result<PullResult>;
    fn create_branch(&self, name: &str, from: &str) -> Result<()>;
    fn current_branch(&self) -> Result<String>;
}
```

### 3.3 Path safety and concurrency (resolved gaps from review)

**Path traversal.** Every `concept_id`/`path` input is resolved and canonicalized against the bundle root *before* any filesystem operation, in `BundleStore`, not in higher layers — this is a Phase 1 requirement, not deferred hardening. Rule: reject any input whose resolved absolute path does not stay strictly under the bundle root. Concretely:
- Reject `concept_id`/`path` containing `..` segments, or that is an absolute path, or that resolves (after joining with bundle root and canonicalizing) outside the bundle root.
- Reject inputs containing a null byte or resolving through a symlink that itself points outside the bundle root.
- This check is shared by every `BundleStore` implementation (FS and Git) since both are ultimately filesystem-backed.

**Concurrency.** Each `BundleRepo` instance (one per registered bundle, held for the server process's lifetime) owns a single write mutex guarding all mutating operations against that bundle: `okf_write_concept`, `okf_delete_concept`, `okf_write_index`, `okf_append_log`, `okf_add_citation`, and all `okf_git_*` operations. Reads are not blocked by it. This closes the read-modify-write race (two overlapping writes to the same concept, or a write racing a commit) within a single server process. Cross-process coordination (two independent server instances writing the same bundle) is explicitly out of scope — see §1.2.

---

## 4. Write / commit / push workflow

Writing to a git-backed bundle is **three distinct tool calls**, never implicit:

1. `okf_write_concept` (or delete/index/log/citation writes) → writes to working tree, `git add`s the path. No commit.
2. `okf_git_commit` → commits currently staged changes with a message. No push.
3. `okf_git_push` → pushes the current branch to a remote.

Rationale:
- Lets several related edits land in one meaningful commit instead of a commit-per-file-write
- Gives a human a checkpoint (`okf_git_status` / `okf_git_diff`) to review before anything is committed or pushed
- Prevents an agent from silently pushing to a shared remote on every edit

**Branch policy**: configurable per bundle.
- `direct`: writes/commits happen on whatever branch is checked out (suitable for a personal/local bundle).
- `session-branch` (default for any bundle with a configured remote): a "session" is the lifetime of the in-memory `BundleRepo` instance — i.e. from server process start (or first access) until the process exits, not a per-MCP-connection concept. If the current branch is the configured default branch (e.g. `main`) at first write, the server creates `okf/agent-session-<timestamp>` and records it in memory; every subsequent write in that process's lifetime reuses the same recorded branch. Push targets that branch. Opening a PR / merging to default is left to the user or CI — the server never merges. On restart, a fresh session branch is created even if a prior one is still unmerged — cleanup of stale `okf/agent-session-*` branches is left to the user/CI (open question, §11.3).

**Auth**: never accepted as a tool argument. Resolved server-side per remote from config (SSH key path or token env var). A tool call can never carry or leak credentials.

**Push safety**: `okf_git_push` never force-pushes. A non-fast-forward rejection (remote has diverged) is surfaced as a structured error containing the local and remote commit SHAs; the caller must `okf_git_pull` (and resolve any conflicts) before retrying, never `--force`.

**Conflicts**: `okf_git_pull` surfaces conflicts as a structured error (list of conflicting paths) rather than attempting resolution. Caller must resolve out-of-band (or via a future `okf_git_abort_pull` / manual tool) before retrying.

---

## 5. Tool catalog

All tools take a `bundle` parameter (registry key) first.

### 5.1 Read / navigate

| Tool | Input | Output | Notes |
|---|---|---|---|
| `okf_list_bundles` | — | `[{name, backend, path, default_branch?}]` | |
| `okf_list_concepts` | `bundle`, `prefix?`, `type?`, `tag?` | `[ConceptId]` | |
| `okf_read_concept` | `bundle`, `concept_id` | `Concept` | 404-style error if missing |
| `okf_read_index` | `bundle`, `path` (dir, `""` = root) | index body (rendered or synthesized) + `okf_version` if root | synthesizes a listing on the fly if `index.md` absent, per spec §6 |
| `okf_search` | `bundle`, `query`, `type?`, `tag?`, `mode?: metadata\|fulltext\|auto` (default `auto`) | ranked `[{concept_id, title, description, score, snippet}]` | `metadata` matches title/description/tags/id only; `fulltext` searches body text too (tier 1 or 2, see §5.5); `auto` uses tier 2 if configured for the bundle, else falls back to tier 1 |
| `okf_reindex_bundle` | `bundle` | `{reindexed: count}` | manual full rebuild of the tier-2 index (see §5.5); escape hatch for out-of-band edits or index corruption |
| `okf_get_backlinks` | `bundle`, `concept_id` | `[ConceptId]` | reverse of link graph |
| `okf_get_graph` | `bundle`, `prefix?` | `{nodes:[...], edges:[{source,target}]}` | resolves both link forms; unresolved links flagged, not dropped |
| `okf_validate_bundle` | `bundle` | `{errors:[...], warnings:[...]}` | errors = only the 3 hard conformance rules; everything else is a warning |

### 5.2 Write

| Tool | Input | Output | Notes |
|---|---|---|---|
| `okf_write_concept` | `bundle`, `concept_id`, `frontmatter` (typed), `body: string` **or** `body_sections: {schema?, examples?, citations?, freeform?}` + `body_sections_mode?: replace\|merge` (default `replace`), `mode: create\|update\|upsert` | `Concept` (post-write) | rejects missing/empty `type`; rejects reserved-basename `concept_id` and path-traversal inputs (§3.3); stamps/bumps `timestamp` unless caller pins one; preserves unknown frontmatter keys on update; `merge` upserts section entries by natural key (§9.2) rather than replacing wholesale; gated by `write_allowlist` if configured (§7) |
| `okf_delete_concept` | `bundle`, `concept_id` | `{deleted: true}` | does not cascade-fix inbound links (broken links are tolerated by spec) |
| `okf_write_index` | `bundle`, `path`, `sections:[{heading, entries:[{title, path, description}]}]`, `okf_version?` (root only) | index body | rejects `okf_version` on non-root path |
| `okf_append_log` | `bundle`, `path`, `date? (default: today)`, `entries:[{label?, text}]` | updated `log.md` body | creates `## YYYY-MM-DD` heading if absent for that date; appends under it; `label` becomes the bold prefix convention if given |
| `okf_add_citation` | `bundle`, `concept_id`, `label`, `target` (url or bundle path) | updated `# Citations` section | creates the section if absent; auto-numbers |

### 5.3 Git-only (present only for bundles configured with `backend: git`)

| Tool | Input | Output | Notes |
|---|---|---|---|
| `okf_git_status` | `bundle` | `{staged:[...], unstaged:[...], untracked:[...], branch}` | |
| `okf_git_diff` | `bundle`, `path?` | unified diff text | |
| `okf_git_commit` | `bundle`, `message`, `author?` | `{sha}` | fails if nothing staged |
| `okf_git_push` | `bundle`, `remote? (default from config)`, `branch? (default: current)` | `{pushed_branch, remote}` | |
| `okf_git_pull` | `bundle`, `remote?`, `branch?` | `{updated: bool, conflicts?: [path]}` | never auto-resolves |
| `okf_git_create_branch` | `bundle`, `name`, `from? (default: current)` | `{branch}` | |

### 5.4 Resources (read-only MCP resource URIs, mirrors §5.1 reads)

- `okf://{bundle}/{concept_id}` → concept (frontmatter + body)
- `okf://{bundle}/_index/{path}` → index listing (rendered or synthesized)

Resources exist alongside the read tools so clients that prefer resource-browsing (vs. explicit tool calls) can navigate a bundle directly.

### 5.5 Search architecture

Full-text search over concept **bodies** matters more than metadata-only search in practice — the questions an agent asks ("how do we compute weekly active users?") are usually answered in body prose, not frontmatter. Two tiers, so bundle size dictates complexity rather than the other way around:

**Tier 1 — grep-based, no index (default for every bundle).**
Scans concept bodies directly at query time via the `grep`/`ignore` crates. No persistent state; always exactly reflects the working tree; trivial to reason about. Adequate up to a few thousand concepts / moderate total text volume.

**Tier 2 — persistent `tantivy` index (opt-in per bundle, `search_index = "tantivy"` in config, for scale).**
Schema: `concept_id` (stored), `type`/`tags` (indexed + faceted), `title`/`description` (indexed, boosted over body), `body` (indexed), `timestamp` (stored). Index lives in a server-managed data directory (e.g. `.okf-index/{bundle}/`) — **never inside the bundle's own tree**, so it's never accidentally committed to a git-backed bundle.

Index maintenance (treated strictly as a derived cache, never a source of truth):
- `okf_write_concept` / `okf_delete_concept` incrementally update or remove that single doc
- `okf_git_pull` diffs old vs. new HEAD (via `git2`) and reindexes only the changed paths — not a full rebuild
- `okf_reindex_bundle` is the manual full-rebuild escape hatch, for out-of-band edits (e.g. someone `git pull`s directly in their own terminal, bypassing the server) or index corruption
- optional `watch = true` config (via the `notify` crate) for FS-backed bundles, so external filesystem edits stay indexed without an explicit reindex call

`okf_search` results always include a **snippet** — a highlighted excerpt around the match (tantivy's built-in snippet generator for tier 2; matched-line-plus-context for tier 1 grep) — so a caller can see why something matched without re-reading the whole concept.

### 5.6 Audit logging (resolved gap: FS backend has no history otherwise)

Git-backed bundles get a history mechanism for free via commits, but commits are coarse (one entry per `okf_git_commit` call, possibly bundling several tool calls) and FS-backed bundles get **no** history at all. The server maintains its own append-only audit log per bundle, independent of both git history and the spec's own `log.md` (which is human-curated content, not a mechanical audit trail):

- Location: server-managed data dir, e.g. `.okf-audit/{bundle}.jsonl` — never inside the bundle's own tree, same reasoning as the tier-2 search index (§5.5).
- One JSON line per mutating tool call: `{timestamp, tool, bundle, target_path, caller, params_summary, result: ok|error}`.
- `caller` is best-effort in v1 — whatever identity the MCP client/transport surfaces, defaulting to `"unknown"` if none. A real auth/identity layer is out of scope for this plan (no MCP-level caller authentication is assumed) but the field is reserved now so it isn't a breaking schema change later.
- Applies to both backends uniformly — for git-backed bundles this is a finer-grained, per-tool-call complement to commit history, not a replacement for it.
- No dedicated read tool in v1; it's an operational log for the person running the server, not something exposed to agents. Add an `okf_read_audit_log` tool later if a use case needs agents to see it.

---

## 6. Validation rules (implementation detail for `okf_validate_bundle` and write-path guards)

**Hard (block the write / reported as `errors`):**
- Missing or empty `type` in frontmatter
- Malformed YAML frontmatter block
- Concept ID's basename (last path segment) is `index` or `log` at *any* directory level — e.g. `foo/index` collides just as much as a root-level `index`, per spec §3.1 ("any level of the hierarchy")
- `concept_id`/`path` fails the path-safety check (§3.3): contains `..`, is absolute, or resolves outside the bundle root
- `okf_version` present in a non-root `index.md`

**Soft (reported as `warnings`, never block):**
- Missing `title` / `description` / `tags` / `timestamp`
- Unknown `type` value
- Unrecognized extra frontmatter keys
- Broken links (target doesn't resolve to an existing concept)
- Missing `index.md` in a directory
- `log.md` entries not under a valid `## YYYY-MM-DD` heading (fixed up on next `okf_append_log` call rather than rejected)

---

## 7. Configuration

```toml
[bundles.sales_catalog]
backend = "git"
path = "/data/bundles/sales_catalog"
remote = "origin"
default_branch = "main"
branch_policy = "session-branch"   # or "direct"
auth = { ssh_key = "/secrets/deploy_key" }   # never passed via tool args
write_allowlist = ["datasets/**", "tables/**"]  # optional path-scoped write ACL

[bundles.local_notes]
backend = "fs"
path = "/data/bundles/local_notes"
```

- `write_allowlist` (Phase 4 — see §8): restricts which concept-ID/path prefixes any mutating tool may touch for a given bundle — `okf_write_concept`, `okf_delete_concept`, `okf_write_index`, `okf_append_log`, and `okf_add_citation` are all gated by it, evaluated against the target file path each tool would write, not just concept writes. Useful once multiple agents/teams share a server.
- Multiple bundles can be registered in one server process; every tool call is scoped by `bundle` name.

---

## 8. Phasing

**Phase 1 — MVP**
- `LocalFsStore` only
- Path-safety checks (§3.3) and per-bundle write mutex (§3.3) — hard requirements from day one, not deferred hardening
- `okf_list_concepts`, `okf_read_concept`, `okf_write_concept`, `okf_delete_concept`
- `okf_read_index` / `okf_write_index` (with root `okf_version` handling)
- `okf_validate_bundle` (3 hard rules + warnings)
- Audit logging (§5.6)
- Resource URIs for read paths

**Phase 2 — Graph, log & search**
- `okf_get_backlinks`, `okf_get_graph` (both link forms resolved)
- `okf_append_log`, `okf_add_citation`
- `okf_search` — metadata fields **and** tier-1 grep-based full-text over body content (§5.5), with snippets

**Phase 3 — Git backend**
- `GitStore` (git2/libgit2), stage-on-write behavior
- `okf_git_status`, `okf_git_diff`, `okf_git_commit`, `okf_git_push`, `okf_git_pull`, `okf_git_create_branch`
- `branch_policy` (session-branch auto-creation)
- Server-side auth resolution (SSH key / token from config, never from tool args)

**Phase 4 — Hardening & scale**
- `write_allowlist` path ACLs per bundle
- Multi-bundle registry polish, bundle hot-reload on config change
- Optional file-watch cache (`notify` crate) for graph/search on large FS-backed bundles
- Tier-2 `tantivy` persistent search index (opt-in, §5.5), `okf_reindex_bundle`, incremental reindex on write/pull

---

## 9. Structured write input

Two separate structuring opportunities here — worth keeping distinct from each other, and from OKF's own `okf_version` field, which is a different kind of "version" than either.

### 9.1 Frontmatter — already structured, not a YAML blob

`okf_write_concept`'s `frontmatter` param is a typed object (`type`, `title`, `description`, `resource`, `tags`, `timestamp`, plus an `extra` map for producer-defined keys) — never a raw YAML string passed through. The server serializes it to a canonical YAML block on write and parses it back to the same struct on read, with `extra` round-tripping any keys it doesn't recognize (per §4.1 of the spec: consumers must preserve unknown keys).

### 9.2 Body — optional structured sections vs. freeform markdown

The body is markdown prose by spec (no structure required), but the three conventional sections — `# Schema`, `# Examples`, `# Citations` — are themselves tabular/list-shaped data that a caller currently has to hand-format as markdown tables/lists. `okf_write_concept` accepts body as **either**:

- a raw `body: string` (full control, current behavior), or
- a structured `body_sections` object:

```json
{
  "schema": [{"column": "order_id", "type": "STRING", "description": "Unique order identifier."}],
  "examples": [{"title": "Basic query", "language": "sql", "code": "SELECT * FROM orders LIMIT 10"}],
  "citations": [{"label": "BigQuery table schema", "target": "https://console.cloud.google.com/bigquery?..."}],
  "freeform": "Any additional prose, appended after the structured sections."
}
```

The server deterministically renders this into the canonical markdown table/list form, so every concept written through this server gets consistently formatted `# Schema` tables, `# Examples` blocks, and numbered `# Citations` — instead of each caller inventing its own markdown table style.

**Replace vs. merge.** A `body_sections` write takes an explicit `body_sections_mode: "replace" | "merge"` (default `"replace"`):
- `replace`: the given sections wholesale replace the corresponding existing section(s). Omitted sections are left untouched; `freeform` (if given) replaces the freeform trailing content.
- `merge`: entries within `schema`/`examples`/`citations` are upserted against existing entries by a natural key (`column` for schema rows, `title` for examples, `label` for citations) — matching entries are updated in place, non-matching ones are appended, and existing entries not mentioned in the call are left alone. This is what makes "patch one schema row" actually possible without the caller resending the whole table.

`okf_read_concept` can optionally return a best-effort `body_sections` parse alongside the raw body text, so a `merge` call can be constructed from a prior read without hand-diffing markdown.

This is a convenience/consistency layer, not a spec extension: the file on disk is still plain, conformant OKF markdown. A consumer with no knowledge of this server (a different agent, a human in a text editor, the reference visualizer) sees nothing different.

### 9.3 Optional per-type schema registry (server-local extension, not part of OKF)

Since `type` values aren't centrally registered by the spec — and consumers must tolerate unknown ones (§4.1, §9) — any per-type validation has to live as **opt-in server config**, not a spec rule:

```toml
[bundles.sales_catalog.type_schemas."BigQuery Table"]
schema_version = 1
required_frontmatter_extra = []
require_body_sections = ["schema"]
```

This lets you declare "in my org, concepts of type `BigQuery Table` should have a `# Schema` section" and get it flagged in `okf_validate_bundle` — but only ever as a `warning`, never a hard error, so bundles or concepts from other producers (or deliberate hand-authored exceptions) still round-trip fine under the spec's permissive conformance model (§9).

Note `schema_version` here is **your own type-registry's version**, unrelated to the bundle's `okf_version` field (§11 of the spec) — the latter is a bundle-format version declared once in the root `index.md`; the former is per-type-definition versioning local to this server's config. Worth not conflating the two in the implementation.

---

## 10. Testing strategy

- Golden fixtures: the three OKF reference sample bundles (GA4 e-commerce, Stack Overflow, Bitcoin public datasets) from `knowledge-catalog/okf/samples` as read-path integration tests.
- Synthetic bundles for write-path tests: reserved-name collisions, missing `type`, root-vs-non-root `index.md` frontmatter, broken links, unknown extra keys (round-trip preservation).
- Git backend: tests against a local bare repo (no network) for commit/push/pull/branch/conflict scenarios.
- Fuzz/property test for frontmatter round-trip: parse → mutate one known field → serialize → unknown keys must be byte-identical to input.
- Path-safety adversarial tests: `..` segments, absolute paths, null bytes, symlinks pointing outside the bundle root — each must be rejected before touching the filesystem (§3.3).
- Concurrency test: two overlapping `okf_write_concept` calls to the same concept must not interleave (assert the write mutex actually serializes them, not just that no panic occurs).

---

## 11. Open questions for next iteration

Resolved in this pass (kept here for traceability, see referenced sections): git-pull conflict handling stays manual/out-of-band (§4); "session" for branch policy now means process-lifetime, not per-connection (§4); write_allowlist now covers all mutating tools, not just concept writes (§7); path traversal and concurrency are now Phase 1 requirements (§3.3); FS-backend audit gap closed via §5.6.

Still open:

1. Stale `okf/agent-session-*` branch cleanup after a PR merges — still explicitly left to the user/CI. Worth a documented convention (e.g. branch name includes enough info to identify safe-to-delete candidates) even if the server itself never deletes branches.
2. Should `okf_delete_concept` optionally scan and report (not fix) concepts that link to the deleted ID, as a courtesy warning?
3. Multi-bundle auth: one credential set per bundle, or a shared credential store keyed by remote host?
4. Tier-1→tier-2 search threshold: pick it by concept count, total body byte size, or just make it purely a manual config toggle the operator flips when grep starts feeling slow?
5. Audit log (§5.6) `caller` identity — is "best-effort from MCP transport, defaulting to unknown" acceptable for v1, or does this plan need a real caller-identity story before Phase 1 given the audit log's main value is knowing *who* wrote what?
