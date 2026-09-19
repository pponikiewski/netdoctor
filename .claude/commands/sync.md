---
description: Sync docs (notes, plan, architecture, README) with work done this session
allowed-tools: Read, Edit, Write, Grep, Glob, Bash(git log:*), Bash(git diff:*), Bash(git status:*)
---

Sync project docs with the work done in this session. Run `git log --oneline -10` to see recent commits, then:

1. **docs/notes.md** — append ONE line per commit made this session that isn't logged yet:
   `- YYYY-MM-DD | <commit subject> | <one-sentence what/why>`
2. **docs/plan.md** — if it exists: check off criteria that were actually VERIFIED this session (a command passed, behavior observed, test green) — not ones merely claimed done. If you only believe something is done without evidence, leave it unchecked and note why. The next session reads this file as fact and won't re-check it. If all are verified, add `STATUS: COMPLETED <date>` at the top
3. **docs/architecture.md** — append an entry ONLY if this session involved an architectural decision (new dependency, schema change, pattern choice, API design). Otherwise do not touch it
4. **CLAUDE.md Stack section** — compare against the project manifest (package.json / pyproject.toml / Cargo.toml / go.mod / *.csproj / pom.xml). If the Stack section contains `[UZUPEŁNIJ` placeholders or has drifted (library added/removed), rewrite ONLY that section to match reality. Same for placeholder commands in the Commands section
5. **docs/_index.md** — update the `updated:` field in frontmatter to today's date. If a milestone was reached (feature completed, phase done), also add one line to the Status section
6. **README.md** — update ONLY when user-facing reality changed this session: completed feature → one bullet in Features; setup/scripts changed → fix Getting Started / Commands table; Tech Stack drifted → sync with CLAUDE.md. Surgical edits only — never rewrite the whole file, never touch the description line
7. **.gitignore** — run `git status --porcelain` and scan untracked files: build artifacts, caches, installer outputs, IDE junk → append matching patterns (one per line, skip if pattern already present). Anything secret-like (.env variants, *.key, *.pem, credentials, DB dumps): add the pattern AND — if such a file is already TRACKED — do NOT touch git history yourself; warn me explicitly and suggest `git rm --cached <file>`
8. **Knowledge Base candidate** — if this session solved a NON-TRIVIAL, reusable problem (a tricky bug, an encoding/config/env gotcha, a pattern worth reusing in other projects — NOT routine feature work), propose ONE knowledge entry: print the filled template (Problem / Rozwiązanie / Dlaczego / snippet / tagi) and tell me to save it as `knowledge/<slug>.md` in the vault. Do NOT write outside this repo yourself — just hand me the ready entry to paste

Then show me a 3-line summary of what was updated and remind me the synced docs are uncommitted. Only docs/, README.md, .gitignore and the CLAUDE.md Stack/Commands sections may be modified. Do NOT commit anything in this mode.
