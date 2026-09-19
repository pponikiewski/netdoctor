---
description: Codebase health audit — finds dead code, duplication, over-engineering and structural mess across the whole project (not just the diff)
argument-hint: [optional: area to focus, e.g. "components" or "lib"]
allowed-tools: Read, Grep, Glob, Bash(git log:*)
---

Health audit of: $ARGUMENTS (if empty, audit the whole project)

Use Grep to trace references and imports before judging anything — do NOT read whole folders blindly. Assess against these categories. For each finding: file:line, why it's a problem, and the concrete fix. Group by category, most impactful first.

1. **Dead code** — unused exports, functions, components, variables; unreachable branches; commented-out blocks left behind. Grep for the identifier across the repo before flagging — an export with zero hits outside its own file is a candidate, but note if it might be a public API or used by tests/other projects
2. **Duplication** — repeated logic that should be one function; copy-pasted components differing only in props; parallel implementations of the same thing
3. **Over-engineering** (Simplicity First) — abstractions with a single caller, premature generalization, config for values that never change, wrapper layers that add no value, interfaces with one implementation
4. **Structural mess** — inconsistent file/folder organization, naming that drifts from project conventions (check CLAUDE.md), files in the wrong place, circular dependencies
4b. **Weak module boundaries** — modules exporting everything instead of a small public interface; a web of small files importing each other across feature lines; logic that should sit behind one entry point spread across many shallow files. Flag where a clear interface would let the internals be changed freely
5. **Stale** — TODO/FIXME older than the feature they reference, obsolete comments contradicting the code, leftover debug logging, unused dependencies in the manifest
6. **Complexity hotspots** — functions doing too much (long, many branches, deep nesting), components mixing concerns (data + UI + side effects)

Output: findings grouped by category, each with severity (high = actively harmful/confusing, medium = worth fixing, low = nice-to-have) and file:line. End with a 2-line summary: biggest win + overall health verdict (CLEAN / SOME DEBT / NEEDS CLEANUP).

This is READ-ONLY assessment — do not delete or modify anything. If I want to act on findings, I will ask for a specific cleanup as a separate task (so each deletion is deliberate and reviewable). Never assume an unused-looking export is safe to remove without confirming references.
