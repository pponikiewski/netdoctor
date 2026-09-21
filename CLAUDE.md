# Project: netdoc

> A Windows desktop app that tells a home user which link broke when "the
> internet drops" — the adapter, the Wi-Fi, the router, or the ISP — and keeps
> enough recorded evidence to prove it to whoever has to fix it.
>
> What that anchors: the app is only worth having if its verdict is
> trustworthy, so **an honest "I could not read this" always beats a confident
> guess**. Several fixes in this repo exist because a failure was being
> reported as a measurement. When a change would make the app claim more than
> it knows, that is the reason to reject it.

## Stack
- **Language:** Rust (edition 2021, rust-version 1.82) — Cargo.toml is the source of truth
- **Build:** Cargo
- **Lint:** clippy (warnings = errors)
- **Format:** rustfmt, pinned by `rustfmt.toml` (`use_small_heuristics = "Max"`) — never run `cargo fmt` with a different config, it reformats the whole tree
- **GUI:** eframe/egui 0.29 + egui_plot (glow backend, default fonts)
- **Storage:** rusqlite 0.32 (bundled SQLite)
- **HTTP:** ureq 2 (tls) — the load test and the self-updater; responses are parsed with serde_json, not `into_json`, which sits behind a default feature Cargo.toml does not name
- **Platform:** windows 0.58 crate (IpHelper, WiFi, WinSock, Registry, Shell) — Windows only
- **Release:** `.github/workflows/release.yml`, triggered by a `v*` tag; toolchain pinned, not `stable`. The tag must match `version` in Cargo.toml or the workflow refuses to publish

## Commands
```bash
cargo build        # install deps
cargo run        # run dev
cargo test        # tests
cargo clippy -- -D warnings        # static check
cargo build --release        # build
```

## Rules
- No `unwrap()`/`expect()` in library code — propagate with `?` and proper error types
- `cargo fmt` before every commit
- Prefer borrowing over cloning; document any intentional clone

## Workflow (complements Karpathy guidelines — do not repeat their rituals)
- Karpathy principles (assumptions, surgical scope, simplicity) are active globally — apply them, don't restate them
- **Mark deliberate simplifications** with a `ponytail:` comment naming the ceiling and the upgrade path, e.g. `// ponytail: in-memory store, swap for Redis if it needs to survive restarts`. A known shortcut reads as intent; an unmarked one reads as a bug
- If an explanation defending a simplification would be longer than the code itself, cut the explanation — prose defending simplicity is complexity smuggled back in
- **Steer, don't prescribe:** when guidance has no single right answer (approach, architecture), give the goal and the why, then trust your judgment for the how — never follow a checklist that doesn't fit the task. When it has a defensibly right shape (format, security), keep the specifics
- **Verification loop (Goal-Driven Execution):** a task is DONE only when `cargo clippy -- -D warnings` and `cargo test` pass AND each acceptance criterion is checked off. Loop autonomously until green — don't report partial success
- **Self-critique before "done":** before presenting any non-trivial output as finished, re-read it as a sceptical reviewer and name at least ONE concrete weakness (missing error handling, untested edge case, unstated assumption). Then either fix it or flag it explicitly. First draft is never the final answer — "I reviewed it and it looks OK" is not self-critique
- **Fast path:** trivial change (typo, one-liner, single-file tweak) → skip planning rigor entirely, just do it and run the static check
- Docs are synced via `/sync`: docs/notes.md (always), docs/plan.md (checkboxes), docs/architecture.md (only on architectural decisions), README (user-facing changes). Remind me if the session did significant work and I haven't synced

## Tools (installed MCP/plugins — use them)
- **Targeted reading:** use Grep/Glob to find the exact symbol or file before reading; never read a whole folder just to see what is there
- **Context7:** when unsure about a library/framework API — fetch current docs instead of guessing from memory
- Never read entire folders; read exact files or symbols
- **Built-in skills:** Claude Code ships `/verify`, `/code-review`, `/run`, `/doctor`. Prefer them where they fit; this project's commands add project-specific workflow on top. Note: this project's `/review` shadows the built-in one — use `/code-review` for stock behavior. Planning: use built-in `/plan` (plan mode, edits disabled), then `/spec` for the structured spec

## Knowledge Base
- A cross-project knowledge base of solved problems lives OUTSIDE this repo (Obsidian vault: knowledge/). It is not in context by default
- When you hit a problem that feels like it might have been solved before (encoding, build config, auth flow, env quirks), ASK me to check the KB before solving from scratch
