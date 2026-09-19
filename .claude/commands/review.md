---
description: Senior-level review of current changes
allowed-tools: Read, Grep, Glob, Bash(git diff:*), Bash(git status:*), Bash(cargo test:*), Bash(cargo clippy:*)
---

Review current changes (`git diff`) like a senior engineer:

1. **Correctness** — matches the spec / intent?
2. **Code quality** — follows the project Rules in CLAUDE.md; inputs validated where they enter the system
3. **Security** — no exposed secrets, auth checked, no injection vectors
4. **Performance** — obvious inefficiencies, N+1 patterns, unnecessary work in hot paths
5. **Tests** — critical paths covered? run them.

Output: ✅ PASS or 🔴 ISSUES — each issue with file:line + suggested fix.
Do not fix anything yourself in this mode.
