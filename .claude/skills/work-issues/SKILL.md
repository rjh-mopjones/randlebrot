---
name: work-issues
allowed-tools: Read, Grep, Glob, Bash(*), Agent, WebSearch
argument-hint: [feature-parent-number] (optional: only process sub-issues under this parent)
description: Discover leaf work items (sub-issues or stand-alone issues), build a dependency graph from GitHub's native sub-issue + dependencies APIs, and spawn parallel implementation agents — one per ready work item in isolated worktrees, each raising a PR
model: claude-opus-4-6
---

# work-issues — Randlebrot issue triage and parallel implementation swarm

You are the Randlebrot issue orchestrator. You will read all open GitHub issues
that have no associated PR, work out which can be implemented in parallel right
now based on the **native sub-issue hierarchy** and **issue dependencies API**,
then spawn an agent swarm to implement them — one agent per ready issue, each
raising a PR when done.

Read `CLAUDE.md` in full before doing anything else. The dependency graph,
workspace crate map, and architecture sections are the source of truth for
what blocks what at the code level:

```bash
cat CLAUDE.md
```

---

## How Randlebrot tracks work (post-skills-refactor)

**Parent issues** are feature trackers. They describe a coherent feature or
capability at a high level. They are NOT work units themselves — agents do
not implement parent issues directly.

**Sub-issues** are concrete work items. Each sub-issue is small enough for one
agent to implement in a single PR. Sub-issues live under a parent via GitHub's
native sub-issues API. A parent shows `sub_issues_summary: {total, completed,
percent_completed}` automatically.

**Stand-alone issues** (no parent, no children) are also work units — used for
one-shot tasks that don't decompose.

**Issue dependencies** replace the old "Depends on: #X" body text. Encoded via
GitHub's native dependencies API: `blocked_by` / `blocking`. Queried via
`/repos/{owner}/{repo}/issues/{N}/dependencies/blocked_by`. The agent is "ready"
if all its blockers are closed.

**Waves are derived, not labelled.** The wave number of an issue is its
topological depth in the dependency graph. A leaf with no blockers is wave 0.
A leaf whose blockers are all in wave 0 is wave 1. Etc. The old `wave-N`
labels have been retired — never create them, never rely on them.

---

## Phase 0 — Discover work items

### 0a. Fetch all open issues (with sub-issue + dependency summaries)

```bash
# --paginate concatenates all pages into a single JSON array, so we get
# the full issue list regardless of repo size. Without it, the response
# is capped at per_page (max 100) on page 1.
gh api /repos/rjh-mopjones/randlebrot/issues \
  --paginate \
  -X GET \
  -f state=open \
  -f per_page=100 \
  > /tmp/rb_all_issues.json

python3 -c "
import json
d = json.load(open('/tmp/rb_all_issues.json'))
# Filter out PRs (the issues endpoint returns both)
d = [i for i in d if 'pull_request' not in i]
print(f'{len(d)} open issues (non-PR)')
json.dump(d, open('/tmp/rb_all_issues.json', 'w'), indent=2)
"
```

### 0b. Fetch all open PRs to find which issues are already in progress

```bash
gh pr list --state open --limit 100 \
  --json number,title,body,headRefName \
  > /tmp/rb_all_prs.json
```

### 0c. Classify every issue (parent / work-unit / claimed)

For each open issue, determine:
1. **Is it a parent?** (has `sub_issues_summary.total > 0`)
2. **Is it a sub-issue?** (has `parent_issue_url != null`)
3. **Is it claimed?** (its number appears in an open PR's branch name or body)

Parents are trackers (skip). Work units are:
- Sub-issues whose parent is open (and not already implemented)
- Stand-alone issues (no parent, no sub-issues)

```bash
python3 << 'EOF'
import json, re

issues = json.load(open('/tmp/rb_all_issues.json'))
prs = json.load(open('/tmp/rb_all_prs.json'))

# Extract issue numbers mentioned in open PR bodies/branches
claimed = set()
for pr in prs:
    text = (pr.get('body') or '') + ' ' + pr.get('headRefName', '')
    for m in re.findall(r'#(\d+)|issue[- ](\d+)', text, re.IGNORECASE):
        num = m[0] or m[1]
        if num:
            claimed.add(int(num))

# Optional filter: if argument is "N", only include sub-issues of parent #N
target_parent = None
args = "$ARGUMENTS".strip()
if args and args.isdigit():
    target_parent = int(args)

parents = set()
work_units = []

for issue in issues:
    num = issue['number']
    summary = issue.get('sub_issues_summary') or {}
    has_children = (summary.get('total') or 0) > 0
    parent_url = issue.get('parent_issue_url')
    parent_num = None
    if parent_url:
        m = re.search(r'/issues/(\d+)$', parent_url)
        if m:
            parent_num = int(m.group(1))

    if has_children:
        parents.add(num)
        continue

    if num in claimed:
        continue

    if target_parent is not None and parent_num != target_parent:
        continue

    work_units.append({
        'number': num,
        'title': issue['title'],
        'parent_number': parent_num,
        'id': issue['id'],
        'labels': [l['name'] for l in issue.get('labels', [])],
        'blocked_by_count': (issue.get('issue_dependencies_summary') or {}).get('blocked_by', 0),
    })

print(f'Open issues (non-PR): {len(issues)}')
print(f'Parents (trackers):   {len(parents)}  -> {sorted(parents)}')
print(f'In open PR:           {len(claimed)}  -> {sorted(claimed)}')
print(f'Work units to triage: {len(work_units)}')
if target_parent is not None:
    print(f'Filter: parent #{target_parent}')
print()
for w in work_units:
    parent = f'  (sub-issue of #{w["parent_number"]})' if w['parent_number'] else '  (stand-alone)'
    blocked = f' [blocked by {w["blocked_by_count"]}]' if w['blocked_by_count'] else ''
    print(f'  #{w["number"]}: {w["title"]}{parent}{blocked}')

with open('/tmp/rb_work_units.json', 'w') as f:
    json.dump(work_units, f, indent=2)
EOF
```

If `/tmp/rb_work_units.json` is empty, print "No open work units." and exit.

---

## Phase 1 — Fetch each work unit's full body + blockers

For each work unit, fetch its full body (for the agent prompt) AND its
`blocked_by` list from the dependencies API:

```bash
python3 << 'EOF'
import json, subprocess

work_units = json.load(open('/tmp/rb_work_units.json'))

enriched = []
dep_api_failures = []
for w in work_units:
    num = w['number']

    # Full body + labels
    result = subprocess.run(
        ['gh', 'issue', 'view', str(num), '--json', 'number,title,body,labels'],
        capture_output=True, text=True
    )
    data = json.loads(result.stdout) if result.stdout.strip() else {}

    # Native dependencies: blocked_by
    result = subprocess.run(
        ['gh', 'api', f'/repos/rjh-mopjones/randlebrot/issues/{num}/dependencies/blocked_by'],
        capture_output=True, text=True
    )
    if result.returncode != 0:
        # Silent failure here would cause agents to run with unmet blockers.
        # Record the failure and treat as "unknown blockers" (sentinel).
        dep_api_failures.append((num, result.stderr.strip()))
        blocked_by_raw = None
    else:
        blocked_by_raw = json.loads(result.stdout) if result.stdout.strip() else []

    if blocked_by_raw is None:
        blocked_by = None  # unknown — downstream will treat as NOT ready
    else:
        blocked_by = [
            {'number': b['number'], 'state': b['state'], 'title': b['title']}
            for b in blocked_by_raw
        ]

    enriched.append({
        **w,
        'body': data.get('body', ''),
        'title': data.get('title', w['title']),
        'blocked_by': blocked_by,
    })

    print(f"#{num}: {w['title']}")
    if blocked_by is None:
        print(f"    ! dependencies API failed — treating as blocked (unknown)")
    elif blocked_by:
        for b in blocked_by:
            marker = '✓' if b['state'] == 'closed' else '⏳'
            print(f"    {marker} blocked by #{b['number']} ({b['state']}): {b['title']}")

if dep_api_failures:
    print(f"\nWARNING: {len(dep_api_failures)} dependency API call(s) failed:")
    for num, err in dep_api_failures:
        print(f"  #{num}: {err}")
    print("  These issues will be treated as blocked (unknown state).")

with open('/tmp/rb_issues_enriched.json', 'w') as f:
    json.dump(enriched, f, indent=2)

print(f"\n{len(enriched)} work units enriched.")
EOF
```

---

## Phase 2 — Build the dependency graph

A work unit is **ready** if ALL its `blocked_by` dependencies are closed.
The native dependencies API returns each blocker's `state` field — that is
the authoritative source, regardless of whether the blocker is in the current
batch, in another feature's sub-issue tree, or claimed by an open PR.

A work unit is **blocked** if any of its `blocked_by` dependencies is still
open, or if the Phase 1 API call to fetch its blockers failed (unknown state).

```bash
python3 << 'EOF'
import json

enriched = json.load(open('/tmp/rb_issues_enriched.json'))

analysis = {}
for w in enriched:
    num = w['number']
    blocked_by = w.get('blocked_by')

    if blocked_by is None:
        # Phase 1 failed to fetch — treat as blocked with unknown reason.
        is_ready = False
        blockers_still_open = []
        unknown = True
    else:
        blockers_still_open = [
            b for b in blocked_by if b['state'] == 'open'
        ]
        is_ready = len(blockers_still_open) == 0
        unknown = False

    analysis[num] = {
        **w,
        'blockers_still_open': blockers_still_open,
        'ready': is_ready,
        'blockers_unknown': unknown,
    }

ready_list = [w for w in analysis.values() if w['ready']]
blocked_list = [w for w in analysis.values() if not w['ready']]

print("\n=== READY — CAN START NOW ===")
for w in sorted(ready_list, key=lambda x: x['number']):
    parent = f' (sub-issue of #{w["parent_number"]})' if w['parent_number'] else ''
    print(f"  #{w['number']}: {w['title']}{parent}")

print("\n=== BLOCKED — waiting on open blockers ===")
for w in sorted(blocked_list, key=lambda x: x['number']):
    parent = f' (sub-issue of #{w["parent_number"]})' if w['parent_number'] else ''
    print(f"  #{w['number']}: {w['title']}{parent}")
    if w['blockers_unknown']:
        print(f"      blocked by: (unknown — dependencies API call failed)")
    else:
        deps = ', '.join(f"#{b['number']}" for b in w['blockers_still_open'])
        print(f"      blocked by: {deps}")

with open('/tmp/rb_issues_ready.json', 'w') as f:
    json.dump(ready_list, f, indent=2)

with open('/tmp/rb_issues_all.json', 'w') as f:
    json.dump(list(analysis.values()), f, indent=2)

print(f"\n{len(ready_list)} ready, {len(blocked_list)} blocked.")
EOF
```

If `/tmp/rb_issues_ready.json` is empty, print which issues are blocking and exit.

---

## Confirmation gate — STOP HERE and ask the user

Present a clear summary and ask for explicit confirmation before spawning any
agents. Do not proceed to Phase 3 automatically.

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  READY TO LAUNCH — work-issues swarm
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  Launching now: N agents

  Issue   Parent    Title                          Why ready
  ──────  ────────  ─────────────────────────────  ────────────────────
  #NN     #PP       ...                            no blockers / all closed
  #NN     (stand)   ...                            no blockers
  ...

  Blocked (waiting on open issues in this batch):
  #NN     #PP       ...  — blocked by: #NN, #NN
  ...

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  Each agent will: implement the issue → cargo check/clippy/test
  → update CLAUDE.md + Obsidian docs → raise PR that closes the sub-issue.

  Proceed? (yes / no / adjust)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

Wait for the user's response. Do not proceed automatically.

- **yes** → Phase 3
- **no** → "Aborted." exit
- **adjust** → parse intent, re-display

---

## Phase 3 — Spawn parallel implementation agents

For each ready work unit, spawn an Agent with `isolation: "worktree"` and
`run_in_background: true`.

**IMPORTANT — Spec files**: Before constructing the agent prompt, check if a spec
file exists for this issue:

```bash
ls specs/<NUMBER>-*.md 2>/dev/null
```

If a spec file exists, the spec IS the agent prompt. Read the spec file and include
its full contents in the agent prompt instead of the GitHub issue body. The spec
contains exact code, file paths, verification commands, and constraints — it is
more precise than the issue body.

If no spec file exists, fall back to the issue body (legacy workflow).

Agent prompt template:

```
You are a Randlebrot implementation agent. Your task is to implement issue #<NUMBER>
and raise a pull request that closes it.

Issue title: <title>
Parent issue: #<parent_number> (or "stand-alone")

## Specification

<IF SPEC FILE EXISTS: include full contents of specs/<NUMBER>-*.md>
<IF NO SPEC FILE: include the issue body>

---

## Step 0 — Read context

Read CLAUDE.md in full — it is the authoritative architecture guide. Pay special
attention to:
- Workspace Crate Map and Dependency Graph
- Architecture (chunk pipeline, noise hierarchy, three-domain split)
- World Rules (never violate)
- Conventions

If this issue has a parent (a feature tracker), read the parent's body too:
```bash
gh issue view <parent_number> --json title,body
```

If the spec has a "Documentation Updates" or "Constraints" section, follow those
exactly.

## Step 1 — Create branch

```bash
git checkout -b issue-<NUMBER>/<short-slug>
```

## Step 2 — Implement

Follow Randlebrot conventions from CLAUDE.md:
- Bevy systems in `systems/` submodules within each crate
- Plugin struct + registration in each crate's `lib.rs`
- `SystemSet` for ordering
- RON for authored data
- No ECS queries wider than 3 components without a documented reason
- `&impl Trait` over `Box<dyn Trait>`
- Noise-only code does not depend on Bevy rendering
- Bevy 0.18 pinned, `workspace = true` for bevy sub-crates

If creating a new crate: add to workspace members, update CLAUDE.md crate map +
dependency graph.

## Step 3 — Update documentation

Every issue lists which docs to update. Mandatory:
- **CLAUDE.md** at `/Users/roryhedderman/Documents/IdeaProjects/Rust/randlebrot/CLAUDE.md`
- **Obsidian** at `/Users/roryhedderman/Documents/mop-jones-brain/Notes/Margin's Grip - Randlebrot Engine.md`

If the sandbox blocks writes to Obsidian (file outside the worktree), note it
in the PR body — the orchestrator will handle it separately. Do NOT fail the PR.

## Step 4 — Tests

Always use `--workspace` to catch cross-crate breakage before raising the PR.
An agent working in `rb_world` that only runs `cargo check` on the current
package will miss failures in `rb_editor` or `src/main.rs` that CI will catch.

```bash
cargo check --workspace 2>&1
cargo clippy --workspace --all-targets -- -D warnings 2>&1
cargo test --workspace 2>&1
```

Fix every error and warning. For tests requiring a display or real artifacts,
mark `#[ignore]` with a reason.

## Step 5 — Commit and push

```bash
git add -A
git commit -m "feat(<slug>): <title>

Closes #<NUMBER>."
git push origin issue-<NUMBER>/<short-slug>
```

## Step 6 — Raise the PR

Copy labels from the issue (NOT wave labels — those are retired):
```bash
gh issue view <NUMBER> --json labels --jq '.labels[].name' | grep -v '^wave-'
```

```bash
gh pr create \
  --title "feat: <title>" \
  --label "<labels>" \
  --body "$(cat <<'PREOF'
## Summary
Closes #<NUMBER>.

## What's included
- ...
- Tests: ...
- Doc updates: ...

## Acceptance Criteria
<paste from issue>

🤖 Generated with [Claude Code](https://claude.ai/claude-code)
PREOF
)" \
  --base main
```

When the PR closes the sub-issue, GitHub automatically updates the parent's
`sub_issues_summary.completed` counter.

Print: "Issue #<NUMBER> done — PR raised."
```

After all agents complete, proceed to Phase 4.

---

## Phase 4 — Collect results

```
══════════════════════════════════════════════════════════════
  WORK-ISSUES SUMMARY
══════════════════════════════════════════════════════════════

  PRs raised:
    #NN  <title>  → PR #<num> <url>  (sub-issue of #<parent>)
    #NN  <title>  → PR #<num> <url>  (stand-alone)

  Failed:
    #NN  <title>  — <reason>

  Parent progress:
    #<parent> "<title>"  →  N/M sub-issues complete

  Next wave — unblocked once these PRs merge (re-run /work-issues):
    #NN  <title>  — was blocked by #NN

══════════════════════════════════════════════════════════════
```

For failed agents, print the last few lines of their output for diagnosis.

---

## Rules for all agents

- **Read CLAUDE.md fully.** Authoritative spec.
- **One issue per agent.** Never implement more than what the issue asks.
- **Documentation updates are mandatory.** Every issue specifies which files.
- **`cargo check` must pass** before raising the PR.
- **If the issue is ambiguous**, implement the most conservative interpretation
  and document the ambiguity in the PR description.
- **Never violate World Rules** from CLAUDE.md.
- **Always use `--release`** for any cargo run commands that generate world data.
- **Never create `wave-N` labels** — waves are derived from dependencies, not tagged.
- **When the PR closes a sub-issue**, GitHub auto-updates the parent's progress.
- **If the sandbox blocks Obsidian writes**, note it in the PR body and move on —
  don't fail the PR over an external file.

---

## Reference: GitHub sub-issue + dependency APIs (verified)

**Sub-issues** (parent ↔ child hierarchy):
- `GET  /repos/{owner}/{repo}/issues/{N}/sub_issues` — list children
- `POST /repos/{owner}/{repo}/issues/{N}/sub_issues` — add child (body: `sub_issue_id=<int>`, `replace_parent=<bool>`)
- `DELETE /repos/{owner}/{repo}/issues/{N}/sub_issue` — unlink
- Child exposes `parent_issue_url` for upward traversal
- Parent exposes `sub_issues_summary: {total, completed, percent_completed}`

**Dependencies** (blocks/blocked-by):
- `GET /repos/{owner}/{repo}/issues/{N}/dependencies/blocked_by` — list blockers
- `GET /repos/{owner}/{repo}/issues/{N}/dependencies/blocking` — list what this blocks
- `POST /repos/{owner}/{repo}/issues/{N}/dependencies/blocked_by` — add blocker (body: `issue_id=<int>`)
- `DELETE /repos/{owner}/{repo}/issues/{N}/dependencies/blocked_by/{issue_id}` — remove (use the integer id, not the issue number)
- Summary on any issue: `issue_dependencies_summary: {blocked_by, total_blocked_by, blocking, total_blocking}`

**Quirks**:
- `sub_issue_id` and `issue_id` are the integer `id` field, NOT the issue number. Use `gh api -F` (typed) not `-f` (string).
- DELETE on dependencies takes the integer id in the URL path: `DELETE /issues/N/dependencies/blocked_by/<id>`.
- DELETE on sub-issues is **the opposite shape**: `DELETE /issues/N/sub_issue` (singular path segment!) takes `sub_issue_id` in the request body, not the URL. Watch out for the plural/singular mismatch with the POST endpoint.
- The list endpoint (`GET /repos/{owner}/{repo}/issues`) returns `sub_issues_summary` and `parent_issue_url` fields inline, but `issue_dependencies_summary` may require the single-issue endpoint. Phase 1 always fetches blockers via the dedicated `/dependencies/blocked_by` endpoint, so this does not affect correctness.
