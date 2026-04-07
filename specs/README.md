# Specs

Structured specification files for work items. Each spec is the single source of truth for what an implementation agent should build. GitHub issues track STATUS (open/closed); specs contain CONTENT (what, how, verification).

## Format

```markdown
---
issue: 43                              # GitHub issue number
title: Short imperative title
crates: [rb_noise]                     # which crate(s) this touches
modifies:                              # exact file paths
  - crates/rb_noise/src/derived/mod.rs
  - crates/rb_noise/src/biome_map.rs
removes:                               # files/functions to delete (optional)
  - src/commands/launch.rs::build_local_heightmap
depends_on: [42]                       # spec numbers this blocks on (optional)
---

## Goal
One paragraph: what and why.

## Root Cause (for bugs)
The technical explanation of why the current code is wrong.

## Implementation
Exact code to write, exact file locations, exact line numbers.
No ambiguity — the agent copies this and adapts.

## Verification
Bash commands to run that prove the fix works. Expected output.

## Boundary Test (if applicable)
A test that catches the most likely regression.

## Constraints
Non-negotiable rules (World Rules, perf targets, compatibility).
```

## Rules

1. **One spec per issue.** File name: `<issue-number>-<slug>.md`.
2. **The spec IS the agent prompt.** It must contain enough detail that an agent can implement it without reading the issue body.
3. **GitHub issue body is SHORT** — title + one sentence + link to the spec file.
4. **Code samples must compile.** Don't write pseudocode.
5. **Verification must be runnable.** `bash` blocks that produce checkable output.
6. **Specs live in the repo.** They're versioned, reviewed in PRs, and evolve with the code.
7. **CLAUDE.md is the architecture truth.** Specs reference it for constraints, don't duplicate.
8. **Obsidian vault is the design truth.** Specs reference it for game design context.

## Workflow

1. `/explore-feature` designs the feature → creates GitHub issue + spec file
2. `/work-issues` discovers open issues → loads the spec file for each → sends spec as agent prompt
3. Agent implements from the spec → raises PR
4. `/review-prs` reviews the PR against the spec's constraints and verification
5. PR merged → issue closed → spec stays in repo as documentation
