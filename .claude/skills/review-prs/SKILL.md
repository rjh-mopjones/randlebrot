---
name: review-prs
allowed-tools: Read, Grep, Glob, Bash(*), Agent, LSP, WebSearch
argument-hint: [pr-number] (optional: only review this specific PR)
description: Discover open PRs, check merge conflicts and CI status, spawn parallel review+fix agents — one per unreviewed (or changed) PR. Each agent fixes conflicts, fixes CI, reviews the PR, posts inline comments, and applies all fixes.
model: claude-opus-4-6
---

# review-prs — Randlebrot PR review, fix, and health agent

You are the Randlebrot PR review orchestrator. Your job is to:

1. Discover all open PRs (merge state, CI status)
2. For each PR, check whether it has already been reviewed by this command at the current SHA
3. Spawn parallel review agents — one per unreviewed (or changed) PR
4. Each agent fixes merge conflicts, fixes CI failures, reviews the PR against Randlebrot's World Rules and architecture, posts inline GitHub comments, applies all fixes, and force-pushes with lease

Read `CLAUDE.md` before doing anything else:
```bash
cat CLAUDE.md
```

---

## Phase 0 — Discover open PRs and their review state

```bash
# Get all open PRs with metadata, merge status, and CI status
gh pr list --state open --limit 100 \
  --json number,title,headRefName,headRefOid,body,comments,mergeable,mergeStateStatus,statusCheckRollup \
  > /tmp/rb_open_prs.json

cat /tmp/rb_open_prs.json
```

For each PR, check whether a `review-prs` bot comment already exists at the current SHA:

```bash
python3 << 'EOF'
import json, subprocess, sys

prs = json.load(open('/tmp/rb_open_prs.json'))
SENTINEL = '<!-- review-prs-bot -->'

# Optional: filter to a single PR number if passed as argument
target_pr = None
args = "$ARGUMENTS".strip()
if args and args.isdigit():
    target_pr = int(args)

needs_review = []
already_reviewed = []

for pr in prs:
    num = pr['number']
    if target_pr and num != target_pr:
        continue

    merge = pr.get('mergeable', 'UNKNOWN')          # MERGEABLE | CONFLICTING | UNKNOWN
    state = pr.get('mergeStateStatus', 'UNKNOWN')   # CLEAN | DIRTY | BLOCKED | BEHIND | UNKNOWN

    checks      = pr.get('statusCheckRollup') or []
    failing     = [c for c in checks if c.get('conclusion') in ('FAILURE', 'ERROR', 'TIMED_OUT')]
    check_names = [c.get('name', c.get('context', '?')) for c in failing]

    has_conflict = merge == 'CONFLICTING'
    has_failures = bool(failing)

    health_notes = []
    if has_conflict:  health_notes.append('CONFLICT')
    if has_failures:  health_notes.append('CI:' + ','.join(check_names))
    health_str = ' | '.join(health_notes) if health_notes else 'healthy'

    # Fetch all comments on this PR
    result = subprocess.run(
        ['gh', 'pr', 'view', str(num), '--json', 'comments', '--jq', '.comments[].body'],
        capture_output=True, text=True
    )
    comments = result.stdout
    if SENTINEL in comments:
        review_lines = [l for l in comments.split('\n') if 'review-prs-sha:' in l]
        if review_lines:
            last_sha = review_lines[-1].split('review-prs-sha:')[-1].strip()
            current_sha = pr['headRefOid']
            if last_sha == current_sha and not has_conflict and not has_failures:
                already_reviewed.append((num, pr['title'], 'no new commits, healthy'))
            else:
                reason_parts = []
                if last_sha != current_sha:
                    reason_parts.append(f're-review: HEAD changed {last_sha[:7]}→{current_sha[:7]}')
                if has_conflict or has_failures:
                    reason_parts.append(health_str)
                needs_review.append((num, pr['title'], pr['headRefName'], current_sha,
                                     ' + '.join(reason_parts) if reason_parts else 'initial review',
                                     has_conflict, check_names))
        else:
            needs_review.append((num, pr['title'], pr['headRefName'], pr['headRefOid'],
                                 f'initial review ({health_str})', has_conflict, check_names))
    else:
        needs_review.append((num, pr['title'], pr['headRefName'], pr['headRefOid'],
                             f'initial review ({health_str})', has_conflict, check_names))

print("=== SKIPPING (already reviewed at current SHA, healthy) ===")
for num, title, reason in already_reviewed:
    print(f"  PR #{num}: {title} — {reason}")

print("\n=== WILL REVIEW ===")
for num, title, branch, sha, reason, conflict, ci_fails in needs_review:
    print(f"  PR #{num}: {title} [{branch}] @ {sha[:7]} — {reason}")

with open('/tmp/rb_prs_to_review.json', 'w') as f:
    json.dump([
        {'number': num, 'title': title, 'branch': branch, 'sha': sha, 'reason': reason,
         'has_conflict': conflict, 'failing_checks': ci_fails}
        for num, title, branch, sha, reason, conflict, ci_fails in needs_review
    ], f, indent=2)

print(f"\n{len(needs_review)} PRs to review, {len(already_reviewed)} skipped.")
EOF
```

If `/tmp/rb_prs_to_review.json` is empty, print "All PRs are up to date." and exit.

---

## Confirmation gate — STOP HERE and ask the user before proceeding

Present the work list to the user and ask for explicit confirmation before spawning agents:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  READY TO REVIEW — review-prs swarm
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  Launching: N review agents (parallel, isolated worktrees)

  PR #NN: <title>
    Branch: <branch>   HEAD: <sha-short>
    Status: <reason>

  Each agent will:
    1. Read context (CLAUDE.md + PR diff + existing comments)
    2. Fix merge conflicts with main (rebase)
    3. Fix CI failures (cargo check/clippy/test)
    4. Review against Randlebrot World Rules and architecture
    5. Post inline + summary comments on new issues
    6. Apply fixes for ALL issues (blocker, should-fix, nice-to-have)
    7. Commit and force-push with lease

  Proceed? (yes / no / adjust)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

Wait for the user's response. Do not proceed automatically.

---

## Phase 1 — Spawn one review+fix agent per PR

For each PR in `/tmp/rb_prs_to_review.json`, spawn an Agent using the Agent tool with
`isolation: "worktree"` and `run_in_background: true`. The agent prompt should be:

```
You are a Randlebrot PR review, fix, and health agent. You are responsible for PR #{number}
({title}).

Branch: {branch}
Current HEAD: {sha}
Review reason: {reason}
Has merge conflict: {has_conflict}
Failing CI checks: {failing_checks}

Your job has six phases: READ, MERGE-FIX, CI-FIX, REVIEW, COMMENT, FIX.
Complete all six before exiting.

**IMPORTANT:** Use the LSP tool (rust-analyzer) throughout. Before and after every code fix,
use `LSP hover`, `LSP goToDefinition`, and `LSP documentSymbol` to verify types, signatures,
and symbol existence. Never guess at type signatures — ask the LSP.

---

## PHASE R — Read context

Read the authoritative project guide:
```bash
cat CLAUDE.md
```

Pay special attention to:
- **World Rules** — tidally locked physics, no green, no fossil fuels, DeterSim determinism
- **Terrain Quality Requirements** — mountains at plate boundaries, dendritic erosion, 45°C gate, no vegetation in bottom 25%, no vegetation within 10% of sub-stellar
- **Three-Domain Architecture** — TerrainGen/LifeGen/SceneGen boundaries
- **Workspace Crate Map and Dependency Graph** — rb_core → rb_noise → rb_world → ...
- **Conventions** — Bevy systems in `systems/` submodules, plugin registration in `lib.rs`, SystemSet for ordering, RON for authored data, `&impl Trait` over `Box<dyn Trait>`

Fetch the PR metadata, existing review comments, and diff:
```bash
gh pr view {number} --json title,body,comments,files
gh pr diff {number}
git fetch origin {branch}
git checkout {branch}
```

Read ALL existing comments and build a list of issues already raised. This is critical —
do not re-raise issues that are already commented and not yet fixed.

```bash
gh pr view {number} --json comments --jq '.comments[] | "--- \\(.author.login) ---\\n\\(.body)"'
```

Identify:
- Issues already raised and FIXED (comment exists, fix appears in subsequent commits)
- Issues already raised but NOT YET FIXED
- Issues that are NEW (not yet commented on at all)

---

## PHASE M — Fix merge conflicts

Check if the branch has conflicts with main:
```bash
git fetch origin main
git merge-base HEAD origin/main
git diff HEAD...origin/main --name-only
```

If the branch is behind main or has conflicts, rebase onto main:
```bash
git rebase origin/main
```

**If the rebase hits conflicts, resolve them file by file using these rules:**

##### `Cargo.toml` workspace members
The `members` array in the root `Cargo.toml` is the most common conflict.
Both sides added different crates — the correct resolution is ALWAYS to include
ALL entries from both sides, in alphabetical order.

When you see:
```
<<<<<<< HEAD
    "crates/rb_artifacts",
=======
    "crates/rb_editor",
>>>>>>> origin/main
```

Resolve to include both:
```toml
    "crates/rb_artifacts",
    "crates/rb_editor",
```

##### `Cargo.lock` conflicts
Never manually resolve. After resolving `Cargo.toml`:
```bash
rm Cargo.lock
cargo generate-lockfile 2>&1 | tail -5
```

##### `.github/workflows/ci.yml` conflicts
The canonical ci.yml includes the Bevy system dependency install step
(`libudev-dev`, `libasound2-dev`, `libwayland-dev`, etc.). If one side has it
and the other doesn't, keep the version that has it. Never drop system deps.

##### `CLAUDE.md` / `README.md` conflicts
Documentation files. Read both sides, merge the content manually — keep all new
sections from both sides. Never drop content added by either side.

##### `src/main.rs` conflicts
`src/main.rs` is the most volatile file in Randlebrot (~3000 lines, touched by
almost every feature). Conflicts here are common. Rules:
- Keep ALL new functions/systems from both sides
- For the `main()` dispatch logic (CLI parsing), keep the union of subcommand handlers
- For AppPhase/state transitions, keep both sets of systems registered
- If two sides register conflicting Bevy resources with the same name, that's a real
  bug — STOP and report to the user

##### After resolving each conflict file:
```bash
git add <resolved-file>
```

Continue the rebase:
```bash
git rebase --continue
```

If the rebase cannot be completed cleanly, abort and use merge instead:
```bash
git rebase --abort
git merge origin/main -m "merge: sync with main for PR #{number}"
```

If no conflicts exist and the branch is up to date, skip this phase.

---

## PHASE I — Fix CI failures

After resolving any merge conflicts, verify the branch compiles and passes CI locally.
Run these in order — earlier failures mask later ones:

```bash
cargo check --workspace 2>&1 | tail -20
```

If `cargo check` fails, diagnose and fix. Common Randlebrot compile errors:
- Missing `serde::Deserialize` on a new type that got added to a persistable struct —
  add the derive (BiomeMap, RiverNetwork, LifeGenData, TileType all must be serializable)
- Bevy 0.18 API drift — use workspace-pinned bevy deps, not `latest`
- `Arc<T>` fields on serializable structs — must be `#[serde(skip)]` with rebuild strategy
- Missing `Clone`/`Debug` on a noise strategy — strategies must derive both
- Image types (`image::DynamicImage`, `RgbaImage`) in serializable structs — must be skipped
- Bevy `Handle<T>` in serializable structs — must be skipped
- NoiseBackend mismatch — GPU path must have CPU fallback
- Missing `#[derive(Component)]` on a new ECS component
- Systems querying more than 3 components without a documented reason — violates convention

**Use the LSP to diagnose:** Before editing, use `LSP hover` on the problematic symbol
to see what the compiler thinks its type is. Use `LSP goToDefinition` to find where
traits are actually defined.

After each fix: `cargo check -p <crate>` to verify before moving on.

Once `cargo check` is clean:
```bash
cargo clippy --workspace -- -D warnings 2>&1 | tail -20
```

Common Randlebrot clippy issues:
- Unused import — remove it
- `unwrap()` in non-test code — convert to `?`, `.expect("reason")`, or graceful handling
- `panic!` in hot paths (noise generation, chunk streaming) — these run thousands of times
- Redundant `.clone()` on `ChunkCoord`/`TileCoord` (they're `Copy`)
- `dead_code` on unused test helpers — delete or gate with `#[cfg(test)]`
- Shadowing a variable with the same name and type — rename

Once clippy is clean:
```bash
cargo test --workspace 2>&1 | tail -20
```

If tests fail, read the test, understand why it fails, fix either the implementation
or the test. **Never delete a test to make CI pass.** Especially watch for:
- `nothing_green_above_45c` — if this fails, the temperature/biome model is broken
- `no_vegetation_in_bottom_25_percent` — sub-stellar region correctness
- `no_vegetation_near_sub_stellar` — 10% radius hard exclusion

These are World Rules tests. If they fail, you've introduced a real bug.

If CI failures are caused by missing system dependencies (Bevy needs `libudev-dev`,
`libasound2-dev`, etc.), check `.github/workflows/ci.yml` and add the install step if missing.

Iterate until all three (`cargo check`, `cargo clippy`, `cargo test`) pass cleanly.

If no CI failures exist, skip this phase.

---

## PHASE V — Review the code

Read every changed file in the PR:
```bash
gh pr diff {number} --name-only | while read f; do
  echo "=== $f ==="
  cat "$f" 2>/dev/null || echo "(deleted)"
done
```

Review against these Randlebrot-specific criteria:

**World Rules compliance (from CLAUDE.md — non-negotiable)**
- Any hardcoded green colors in biomes/flora? → violation (planet is red-giant lit)
- Any day/night cycle logic, seasons, or latitude-based climate? → violation (tidally locked)
- Any temperature noise strategy? → violation (temperature is always derived from light level)
- Any fossil fuel, plastic, asphalt references in flora/materials/economy? → violation
- Any sub-stellar point assumption other than `(0.5, 1.0)` without explicit reason? → check
- Any live tick-based simulation loop? → violation (DeterSim is `f(seed, T)`)

**Terrain Quality Requirements (from CLAUDE.md — do not regress)**
- Changes to `derive_peaks_valleys`? Check that cubic stress envelope is preserved
- Changes to erosion sim? Check iteration count (~120) and fluvial/tectonic balance
- Changes to river routing? Check D8 uses eroded heightmap
- Changes to lapse rate or temperature? Check 25% cap preserved
- Any change that could affect the 45°C vegetation gate? Run the gate tests
- Any change that introduces green biomes in the bottom 25% of the map? Run the test

**Three-Domain Architecture (from CLAUDE.md + DOMAIN_ARCHITECTURE.md)**
- Does `rb_world` (LifeGen) import `BiomeMap` directly? It should only use `TerrainQuery` trait
- Does `rb_noise` depend on `rb_world`? Should never — TerrainGen is downstream of nothing
- Does `rb_core` depend on any crate except `bevy_ecs`/`bevy_math`? Should not
- Are generated civilization types stored in `WorldDefinition`? They should be in `LifeGenData`

**Crate dependency graph**
- Check `Cargo.toml` of any new/modified crate against the graph in CLAUDE.md
- New deps added? They must match or extend the documented graph
- Circular deps? Immediate blocker

**Conventions**
- Bevy systems in `systems/` submodules? Or added to top-level `lib.rs`? (convention is submodules)
- Plugin struct in `lib.rs`?
- Using `SystemSet` for ordering, not `.after()` chains?
- ECS queries wider than 3 components without a documented reason?
- `Box<dyn Trait>` used where `&impl Trait` would work?
- Noise code importing Bevy rendering types? (should not — keep noise pure)
- New crate without an `examples/` that visualizes its output?

**Serde / artifact persistence (post-PR #15)**
- New type added to `BiomeMap`/`RiverNetwork`/`LifeGenData` without serde derives?
- Non-serializable field without `#[serde(skip)]` + rebuild strategy?
- `Arc<T>` field that's being serialized rather than skipped?

**Bevy 0.18 specifics**
- `bevy = "0.18"` pinned in `Cargo.toml`?
- Bevy sub-crate deps using `workspace = true`?
- Using deprecated Bevy 0.17 APIs?

**CLI skeleton (post-PR #14)**
- New subcommand added to `main.rs` without clap parsing?
- New headless subcommand that accidentally initializes Bevy?

**Cross-cutting (check against all other open PRs)**
- Does this PR define something that other open PRs depend on?
- Does this PR duplicate a definition that another open PR also adds?
- Does this PR modify the same crate as another open PR? (note rebase risk)

---

## PHASE C — Post GitHub comments

### Categorise every issue

For each issue, determine:
- SEVERITY: 🔴 blocker (won't compile / breaks World Rules / breaks determinism) | 🟡 should-fix (convention violation, clippy warning) | 🟢 nice-to-have (style, minor optimization)
- STATUS:
  - `new` — not commented before
  - `unresolved` — already commented, not yet fixed
  - `fixed` — already commented and fixed in a later commit

Only comment on `new` issues. Do not re-raise `unresolved` issues. Do not comment on `fixed` issues.

### Post inline comments for new issues

For each `new` issue, post an inline comment at the exact file and line:

```bash
gh api --method POST /repos/rjh-mopjones/randlebrot/pulls/{number}/reviews \
  --field commit_id='{sha}' \
  --field event='COMMENT' \
  --field body='<!-- review-prs-bot -->\nreview-prs-sha: {sha}\n\n**Review summary:** N new issues found.' \
  --field 'comments[][path]=<file>' \
  --field 'comments[][line]=<line>' \
  --field 'comments[][body]=<severity emoji> **<issue title>**\n\n<detailed explanation>\n\n<code suggestion if applicable>'
```

Rules for comment bodies:
- Start with severity emoji and bold title
- Explain WHY it is a problem, not just WHAT
- For World Rules violations: cite the specific rule from CLAUDE.md
- For Terrain Quality regressions: explain which requirement is at risk and how to verify
- For compile errors: include the exact fix as a code block
- For convention violations: cite the specific convention from CLAUDE.md
- Keep each comment self-contained — the author must be able to fix it without context

### Post a top-level summary comment

```bash
gh pr comment {number} --body '<!-- review-prs-bot -->
review-prs-sha: {sha}

## Randlebrot automated review

| | Count |
|---|---|
| 🔴 Blockers | N |
| 🟡 Should fix | N |
| 🟢 Nice to have | N |
| ✅ Already resolved | N |
| ⏭️ Previously raised, still open | N |

_New issues are posted as inline comments above._
_Previously raised issues that are still open are not re-commented — see earlier review comments._

<!-- review-prs-bot-end -->'
```

---

## PHASE F — Fix ALL issues and commit

Fix every issue — 🔴 blockers, 🟡 should-fix, AND 🟢 nice-to-have. Every issue raised
in the review must be resolved. Apply fixes directly to the checked-out branch.

Fix order (dependency-respecting):
1. Fixes to `rb_core` first (shared types, traits)
2. Fixes to `rb_noise` (terrain)
3. Fixes to `rb_world` (civ)
4. Fixes to `rb_tilemap`, `rb_entity_spawn`, `rb_player`, `rb_editor`, `rb_artifacts`, `rb_persistence`
5. Fixes to `src/main.rs` last

After every logical group of fixes:
```bash
cargo check -p <affected-crate> 2>&1 | head -30
```

Fix any errors before continuing.

Once all fixes are applied, run the full verification suite:

```bash
cargo check --workspace 2>&1 | tail -5
cargo clippy --workspace -- -D warnings 2>&1 | grep "^error" | wc -l  # Must be 0
cargo test --workspace 2>&1 | tail -10
```

If anything still fails, iterate until clean.

**If the PR touches terrain generation:** regenerate debug layers and spot-check them:
```bash
cargo run --release -p rb_noise --example save_debug_layers
```
Open `debug_layers/derived/Heightmap.png` and `debug_layers/biome.png` — verify no
regressions (mountains at boundaries, no green in bottom 25%, dendritic erosion visible).

Once everything passes:

```bash
git add -A
git commit -m "fix(<crate>): address review comments, resolve conflicts and CI failures

- <bullet per review fix>
- <bullet for conflicts resolved, if any>
- <bullet for CI failures fixed, if any>

Fixes raised by /review-prs bot."

git push --force-with-lease origin {branch}
```

Then post a follow-up comment linking the fix commit:

```bash
FIX_SHA=$(git rev-parse HEAD)
gh pr comment {number} --body "<!-- review-prs-bot -->
review-prs-sha: $FIX_SHA

## Fix commit

Applied fixes for all issues (🔴, 🟡, and 🟢) from the review above.
Resolved merge conflicts: yes/no
Fixed CI failures: yes/no
Commit: \`$FIX_SHA\`

Changes:
$(git show --stat HEAD | tail -n +2)"
```

Print: "PR #{number} review complete."
```

After all agents complete, proceed to Phase 2.

---

## Phase 2 — Collect and print results

After all background agents have completed, collect their results and print a summary:

```
══════════════════════════════════════════════════════════════
  REVIEW-PRS SUMMARY
══════════════════════════════════════════════════════════════

  Reviewed:
    PR #NN  <title>  → <N blockers, N should-fix, N nice-to-have>
    PR #NN  <title>  → <N blockers, N should-fix, N nice-to-have>

  Skipped (already reviewed at current SHA, healthy):
    PR #NN  <title>

  Failed:
    PR #NN  <title>  — <reason>

══════════════════════════════════════════════════════════════
```

For any failed agents, read the last 40 lines of their output for diagnosis.

---

## Re-review behaviour

When `/review-prs` is run again on a PR that has already been reviewed:

- **Same HEAD SHA + healthy (no conflicts, no CI failures):** Agent is skipped entirely.
- **Same HEAD SHA but has conflicts or CI failures:** Agent runs to fix conflicts/CI
  even though code hasn't changed.
- **New HEAD SHA:** Agent runs but reads existing comments first. It will:
  - Fix any merge conflicts with main
  - Fix any CI failures
  - Skip any issue that already has a comment (whether fixed or not)
  - Only raise issues genuinely new in the changed code
  - Note in summary how many previously-raised issues are still open vs resolved
  - Post a new top-level summary with the new SHA for future tracking

Comments accumulate on the PR over time but are never duplicated. Each review pass is
scoped to what's new since the last reviewed SHA.

---

## Randlebrot-specific conflict resolution reference

### Root `Cargo.toml` — workspace members
Each PR adds crates to the `members` array. When PRs diverge from the same base, the
members list conflicts. Resolution: include all entries, alphabetically sorted.

Canonical order:
```toml
members = [
    "crates/rb_artifacts",
    "crates/rb_core",
    "crates/rb_editor",
    "crates/rb_entity_spawn",
    "crates/rb_noise",
    "crates/rb_persistence",
    "crates/rb_player",
    "crates/rb_tilemap",
    "crates/rb_world",
]
```

### `Cargo.lock`
Never manually resolve. Delete and regenerate after fixing `Cargo.toml`:
```bash
rm Cargo.lock && cargo generate-lockfile
```

### `.github/workflows/ci.yml` — Bevy system dependencies
The canonical ci.yml includes the Bevy system dependency install step
(`libudev-dev`, `libasound2-dev`, `libwayland-dev`, `libxkbcommon-dev`). Never drop
these. If one side has them and the other doesn't, keep the version with them.

### `src/main.rs`
This file is volatile — ~3000 lines touched by most features. Conflicts here should
be resolved by keeping the union of all new systems, CLI subcommands, and state
transitions from both sides. If two sides register conflicting Bevy resources with
the same name, STOP and report — that's a real bug.

### `CLAUDE.md`
Documentation. Read both sides, merge content. Keep all new sections from both sides.
Never drop content added by either side. Watch for:
- Crate map additions — keep all new crate entries
- Dependency graph additions — keep all new edges
- New sections (e.g., CLI Workflow, Artifact Storage) — keep both

---

## Rules for all review agents

- **CLAUDE.md is truth.** All architectural judgements defer to it.
- **World Rules are non-negotiable.** Any violation is a 🔴 blocker.
- **Terrain Quality Requirements cannot regress.** If the PR touches terrain code,
  regenerate debug layers and spot-check them.
- **Use the LSP** — always verify types, definitions, and references via rust-analyzer
  before and after making code changes. Never guess at method signatures.
- **`cargo check` before `cargo clippy` before `cargo test`** — earlier failures mask later.
- **Do not re-raise already-commented issues** — check existing comments first.
- **Re-reviews are incremental.** On a re-review, only comment on issues introduced
  since the last reviewed SHA, or issues confirmed still present.
- **One review API call per PR** — batch all inline comments into a single `gh api` call.
- **Fix order matters** — `rb_core` changes before `rb_noise` before `rb_world` before
  `src/main.rs`.
- **No `TODO` in fixes** — if you can't fully fix something, explain in the comment why
  and what the author needs to do.
- **`cargo check`, `cargo clippy`, `cargo test` must all pass** after the fix commit.
- **`--force-with-lease` only** — never `--force`. If rejected because remote was updated
  since the agent started, abort and report rather than overwriting.
- **Never delete tests** to make CI pass — fix the implementation.
- **Never drop content** from either side of a documentation conflict.
- **Rebase over merge** where possible. Fall back to merge only if rebase produces
  unresolvable conflicts.
- **Do not touch files outside the PR's changed set** unless a `rb_core` or `CLAUDE.md`
  fix is strictly required and the PR description says it was intended.
