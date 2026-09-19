---
description: Write a spec with verifiable acceptance criteria before coding (use inside plan mode)
argument-hint: [feature description]
---

Write a spec for: $ARGUMENTS

If not already in plan mode, tell me to enter it first (Shift+Tab or `/plan`) — planning should happen with edits disabled.

Produce a spec structured to feed the Karpathy guidelines (one plan, not two):

1. **Assumptions** (Think Before Coding) — list every assumption you're making; mark uncertain ones with `?` and ask about those (max 2 questions)
2. **Success criteria** (Goal-Driven Execution) — `- [ ]` checklist where each item is VERIFIABLE: a command that must pass, a testable behavior, or a concrete observable state. No vague criteria like "works correctly"
3. **Scope** (Surgical Changes) — exact files to create/modify (find them with Grep/Glob, don't guess paths). Everything outside this list is off-limits during implementation
4. **Simplest approach** (Simplicity First) — one paragraph; if you considered a more complex design, say why you rejected it
5. **Open questions** — end with a numbered list of anything still unresolved that needs my answer. If nothing is unresolved, write "brak"

Keep the plan extremely concise — terse fragments over full sentences, no restating the request, no filler. A plan I actually read beats a thorough one I skim.

Wait for my explicit "go".
After I say "go": FIRST save this spec to `docs/plan.md` (overwrite previous), THEN implement, checking off criteria in that file as you complete them.
No code before "go".
