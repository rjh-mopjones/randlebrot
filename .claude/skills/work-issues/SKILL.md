---
name: work-issues
allowed-tools: Read, Grep, Glob, Bash(*), Agent, WebSearch
argument-hint: [wave-N] (optional: only process issues from this wave)
description: Triage open GitHub issues by wave, build dependency graph, and spawn parallel implementation agents — one per issue in isolated worktrees, each raising a PR
model: claude-opus-4-6
---

# work-issues — Randlebrot issue triage and parallel implementation swarm

You are the Randlebrot issue orchestrator. You will read all open GitHub issues
that have no associated PR, work out which can be implemented in parallel right
now based on wave labels and the crate dependency graph, then spawn an agent
swarm to implement them — one agent per issue, each raising a PR when done.

Read `CLAUDE.md` in full before doing anything else. The dependency graph,
workspace crate map, and architecture sections are the source of truth for
what blocks what:

```bash
cat CLAUDE.md
```

---

## Phase 0 — Discover unimplemented issues

### 0a. Fetch all open issues

```bash
gh issue list --state open --limit 100 \
  --json number,title,body,labels,assignees \
  > /tmp/rb_all_issues.json

cat /tmp/rb_all_issues.json
```

### 0b. Fetch all open PRs to find which issues are already in progress

```bash
gh pr list --state open --limit 100 \
  --json number,title,body,headRefName \
  > /tmp/rb_all_prs.json
```

### 0c. Cross-reference to find unimplemented issues

```bash
python3 << 'EOF'
import json, re, sys

issues = json.load(open('/tmp/rb_all_issues.json'))
prs    = json.load(open('/tmp/rb_all_prs.json'))

# Extract issue numbers mentioned in open PR bodies/branches
claimed = set()
for pr in prs:
    text = (pr.get('body') or '') + ' ' + pr.get('headRefName', '')
    for m in re.findall(r'#(\d+)|issue[- ](\d+)', text, re.IGNORECASE):
        num = m[0] or m[1]
        if num:
            claimed.add(int(num))

# Filter: if user passed a wave argument, only include that wave
wave_filter = None
args = "$ARGUMENTS".strip()
if args and args.startswith("wave-"):
    wave_filter = args

unimplemented = []
for issue in issues:
    if issue['number'] in claimed:
        continue
    labels = [l['name'] for l in issue.get('labels', [])]
    if wave_filter and wave_filter not in labels:
        continue
    unimplemented.append(issue)

print(f"Open issues:       {len(issues)}")
print(f"Already in PR:     {len(claimed)}")
print(f"Unimplemented:     {len(unimplemented)}")
if wave_filter:
    print(f"Wave filter:       {wave_filter}")
print()
for i in unimplemented:
    labels = [l['name'] for l in i.get('labels', [])]
    print(f"  #{i['number']}: {i['title']}  [{', '.join(labels)}]")

with open('/tmp/rb_unimplemented_issues.json', 'w') as f:
    json.dump(unimplemented, f, indent=2)
EOF
```

If there are no unimplemented issues, print "All issues have open PRs." and exit.

---

## Phase 1 — Read and understand every unimplemented issue

For each issue in `/tmp/rb_unimplemented_issues.json`, fetch its full body:

```bash
python3 << 'EOF'
import json, subprocess

issues = json.load(open('/tmp/rb_unimplemented_issues.json'))

enriched = []
for issue in issues:
    num = issue['number']
    result = subprocess.run(
        ['gh', 'issue', 'view', str(num), '--json',
         'number,title,body,labels,comments'],
        capture_output=True, text=True
    )
    data = json.loads(result.stdout) if result.stdout.strip() else issue
    enriched.append(data)
    print(f"\n{'='*60}")
    print(f"Issue #{num}: {issue['title']}")
    print(f"{'='*60}")
    print(data.get('body', '(no body)'))

with open('/tmp/rb_issues_enriched.json', 'w') as f:
    json.dump(enriched, f, indent=2)
EOF
```

---

## Phase 2 — Build the dependency graph

Read the enriched issues and the current codebase state, then determine what
blocks what. Think through this carefully — getting the dependency graph wrong
means agents will try to implement something whose dependencies don't exist yet.

**CRITICAL — inter-issue dependencies:** Two issues that are both individually
unblocked by `main` may still depend on each other. For example, if issue #A
adds serde derives and issue #B creates rb_artifacts (which needs those derives),
then B depends on A — they cannot run in parallel even though neither is blocked
by something already on `main`.

### 2a. Inventory the current codebase state

```bash
# Check which crates exist and their implementation status
python3 << 'EOF'
import os, pathlib

workspace_root = pathlib.Path('crates')

status = {}
for toml in sorted(workspace_root.glob('*/Cargo.toml')):
    crate_dir = toml.parent
    name = crate_dir.name
    src = crate_dir / 'src'

    impl_files = list(src.glob('**/*.rs')) if src.exists() else []
    total_lines = sum(
        len(open(f).readlines())
        for f in impl_files
    )

    if total_lines == 0:
        status[name] = 'EMPTY'
    elif total_lines < 20:
        status[name] = 'STUB'
    else:
        status[name] = f'IMPL ({total_lines} lines)'

for name, state in sorted(status.items()):
    print(f"  {name:<30} {state}")
EOF
```

### 2b. Map each issue to its dependencies

For each unimplemented issue, determine:

1. **What wave label does it have?** (`wave-1` through `wave-5`)
2. **What crate(s) does it touch?** (from issue body)
3. **What does it depend on?** (from "Depends on" field in issue body, plus wave label)
4. **Is everything it depends on already merged to `main`?**
5. **Is any dependency provided by ANOTHER unimplemented issue in this batch?**

```bash
git fetch origin
git log origin/main --oneline | head -20
```

### 2c. Build the full dependency graph (including inter-issue edges)

```bash
python3 << 'EOF'
import json, re

issues = json.load(open('/tmp/rb_issues_enriched.json'))

# Parse wave labels and dependency info from each issue
analysis = {}

for issue in issues:
    num = issue['number']
    body = issue.get('body', '') or ''
    labels = [l['name'] for l in issue.get('labels', [])]

    # Extract wave from labels
    wave = -1
    for label in labels:
        if label.startswith('wave-'):
            try:
                wave = int(label.split('-')[1])
            except ValueError:
                pass

    # Extract "Depends on" from body
    depends_on_issues = []
    for m in re.findall(r'Depends on[:\s]*(.+?)(?:\n|$)', body):
        for ref in re.findall(r'#(\d+)', m):
            depends_on_issues.append(int(ref))

    # Extract "Parallel with" from body
    parallel_with = []
    for m in re.findall(r'Parallel with[:\s]*(.+?)(?:\n|$)', body):
        for ref in re.findall(r'#(\d+)', m):
            parallel_with.append(int(ref))

    # Extract crate(s) from body
    crates = []
    for m in re.findall(r"Crate\(s\)[:\s]*(.+?)(?:\n|$)", body):
        crates.append(m.strip().strip('`'))

    analysis[num] = {
        "title": issue['title'],
        "wave": wave,
        "crates": crates,
        "depends_on": depends_on_issues,
        "parallel_with": parallel_with,
        "labels": labels,
    }

# Check which dependencies are satisfied (merged or not in our batch)
unimplemented_nums = set(analysis.keys())

for num, info in analysis.items():
    blocked_by = []
    for dep in info['depends_on']:
        if dep in unimplemented_nums:
            blocked_by.append(dep)
        # If dep is not in our batch and not in open PRs, it's presumably merged
    info['blocked_by_issues'] = blocked_by
    info['ready'] = len(blocked_by) == 0

# Print the dependency graph
print("\n=== WAVE 0 / READY — CAN START NOW (no blocking deps) ===")
for num, info in sorted(analysis.items()):
    if info['ready']:
        print(f"  Issue #{num}: {info['title']}")
        print(f"    Wave: {info['wave']}, Crates: {', '.join(info['crates'])}")

print("\n=== BLOCKED — WAITING FOR OTHER ISSUES IN THIS BATCH ===")
for num, info in sorted(analysis.items()):
    if not info['ready']:
        deps = ', '.join(f"#{d}" for d in info['blocked_by_issues'])
        print(f"  Issue #{num}: {info['title']}  [wave {info['wave']}]")
        print(f"    Blocked by: {deps}")

# Save analysis
ready = [{'number': num, **info} for num, info in sorted(analysis.items()) if info['ready']]
with open('/tmp/rb_issues_ready.json', 'w') as f:
    json.dump(ready, f, indent=2, default=str)

with open('/tmp/rb_issues_all_waves.json', 'w') as f:
    json.dump(analysis, f, indent=2, default=str)

print(f"\n{len(ready)} issues ready to implement now.")
blocked = len(analysis) - len(ready)
if blocked:
    print(f"{blocked} issues blocked (will become ready after dependencies merge).")
EOF
```

If `/tmp/rb_issues_ready.json` is empty, print which issues are blocking and exit.

---

## Confirmation gate — STOP HERE and ask the user before proceeding

After Phase 2 completes, present a clear summary and ask for explicit confirmation
before spawning any agents. Do not proceed to Phase 3 automatically under any
circumstances.

Present the summary in this exact format:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  READY TO LAUNCH — work-issues swarm
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  Launching now: N agents

  Issue  Crate(s)                        Why ready
  ─────  ──────────────────────────────  ────────────────────
  #NN    rb_<name>                       <one-line reason>
  ...

  Blocked — run /work-issues again after these PRs merge:
  #NN    rb_<name>   [wave N]  — depends on: #NN
  ...

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  Each agent will: implement the issue → cargo check/clippy/test
  → update CLAUDE.md + Obsidian docs → raise PR.

  Proceed? (yes / no / adjust)
    yes    — launch all N agents now
    no     — abort, nothing will be run
    adjust — tell me which issues to include or exclude

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

Wait for the user's response.

- If **"yes"** — proceed to Phase 3.
- If **"no"** — print "Aborted. No agents were spawned." and exit.
- If **"adjust"** — parse the user's intent, re-display summary, ask again.

Do not move on until you have received a clear "yes" equivalent.

---

## Phase 3 — Spawn parallel implementation agents

One agent per ready issue. All agents run concurrently. Each agent runs in its
own git worktree for isolation.

For each ready issue, spawn an Agent with `isolation: "worktree"` and
`run_in_background: true`. The agent prompt for each issue should be:

```
You are a Randlebrot implementation agent. Your task is to implement issue #<NUMBER>
and raise a pull request.

Issue title: <title>
Issue wave: <wave>
Crates to touch: <crates>

Issue description:
<body>

---

## Step 0 — Read context

Read CLAUDE.md in full — it is the authoritative spec for the project. Pay
special attention to:
- The Workspace Crate Map and Dependency Graph
- The Architecture section (chunk pipeline, noise hierarchy, three-domain split)
- World Rules (never violate these)
- Conventions

Read the "Documentation Updates" section of the issue body — you MUST update
the specified files as part of your implementation.

## Step 1 — Create the branch

```bash
git checkout -b issue-<NUMBER>/<short-slug>
```

## Step 2 — Implement the issue

Follow the Randlebrot conventions from CLAUDE.md:
- Bevy systems go in `systems/` submodules within each crate
- Plugin struct and registration in each crate's `lib.rs`
- Use `SystemSet` for ordering
- All authored data serializes as RON
- No ECS queries wider than 3 components without a documented reason
- Prefer `&impl Trait` over `Box<dyn Trait>`
- Noise-only code should not depend on Bevy rendering
- Pin to Bevy 0.18 — use `workspace = true` for bevy sub-crate deps

If creating a new crate:
- Add to workspace members in root `Cargo.toml`
- Follow the existing crate structure pattern
- Add to CLAUDE.md crate map and dependency graph

## Step 3 — Update documentation

Every issue has a "Documentation Updates" section listing which files to update.
These are mandatory — the PR will not be accepted without them:

- **CLAUDE.md** at `/Users/roryhedderman/Documents/IdeaProjects/Rust/randlebrot/CLAUDE.md`
- **Obsidian vault** at `/Users/roryhedderman/Documents/mop-jones-brain/Notes/Margin's Grip - Randlebrot Engine.md`

Read the current state of each file, find the relevant section, and update it.

## Step 4 — Tests

Add tests as specified in the acceptance criteria. Run:

```bash
cargo check 2>&1
cargo clippy -- -D warnings 2>&1
cargo test 2>&1
```

Fix every error and warning before continuing. For tests that require
generated data or a running GPU, mark them `#[ignore]` with a reason.

## Step 5 — Commit and push

```bash
git add -A
git commit -m "feat(<crate-slug>): <title>

Closes #<NUMBER>."

git push origin issue-<NUMBER>/<short-slug>
```

## Step 6 — Raise the PR

Fetch the issue's labels and copy them to the PR:

```bash
gh issue view <NUMBER> --json labels --jq '.labels[].name'
```

```bash
gh pr create \
  --title "feat: <title>" \
  --label "<labels from issue>" \
  --body "$(cat <<'PREOF'
## Summary

Implements — closes #<NUMBER>.

## What's included

- <list what was implemented>
- Tests: <describe test coverage>
- Doc updates: <list which docs were updated>

## Acceptance Criteria from Issue

<paste the acceptance criteria checkboxes>

🤖 Generated with [Claude Code](https://claude.ai/claude-code)
PREOF
)" \
  --base main
```

Print: "Issue #<NUMBER> done — PR raised."
```

After all agents complete, proceed to Phase 4.

---

## Phase 4 — Collect results

After all background agents have completed, load `/tmp/rb_issues_all_waves.json`
for the full wave graph, collect agent results, and present a summary:

```
══════════════════════════════════════════════════════════════
  WORK-ISSUES SUMMARY
══════════════════════════════════════════════════════════════

  PRs raised:
    #NN  <title>  → PR #<pr-number> <url>
    #NN  <title>  → PR #<pr-number> <url>

  Failed:
    #NN  <title>  — <reason>

  Next wave — unblocked once these PRs merge (re-run /work-issues):
    #NN  <title>  [wave N]  — depends on: #NN
    #NN  <title>  [wave N]  — depends on: #NN → #NN (chain)

══════════════════════════════════════════════════════════════
```

For any failed agents, print the last few lines of their output for diagnosis.

---

## Dependency resolution rules for this Randlebrot workspace

### Crate dependency graph (from CLAUDE.md)

```
rb_core          → (none, only bevy_ecs + bevy_math)
rb_noise         → rb_core, noise crate
rb_world         → rb_core, rb_noise
rb_tilemap       → rb_core, rb_world
rb_entity_spawn  → rb_core, rb_world, rb_tilemap
rb_editor        → rb_core, rb_noise, rb_world, rb_tilemap, bevy_egui
rb_player        → rb_core, rb_tilemap
rb_persistence   → rb_core, rb_world, rb_tilemap
rb_artifacts     → rb_core, rb_noise, rb_world
```

### Wave structure for the CLI feature

| Wave | Issues | Can start when |
|------|--------|---------------|
| 1 | CLI skeleton, Serde derives | Immediately — no cross-deps |
| 2 | rb_artifacts crate | After serde derives merged |
| 3 | generate layers, view list/detail | After rb_artifacts + CLI skeleton merged |
| 4 | Layer viewer, generate level | After generate layers merged |
| 5 | launch command, GUI migration | After generate level / rb_artifacts merged |

### Rules

- **Read CLAUDE.md fully.** It is the authoritative spec.
- **One issue per agent.** Never implement more than what the issue asks for.
- **Documentation updates are mandatory.** Every issue specifies which CLAUDE.md
  sections and Obsidian files to update.
- **`cargo check` must pass** before raising the PR.
- **Raise the PR even if some acceptance criteria are incomplete** — document
  what's missing rather than blocking the PR indefinitely.
- **If an issue is ambiguous**, implement the most conservative interpretation
  and document the ambiguity in the PR description.
- **Never violate World Rules** from CLAUDE.md (tidally locked physics, no green,
  no fossil fuels, DeterSim determinism, etc.).
- **Always use `--release`** for any cargo run commands that generate world data.
