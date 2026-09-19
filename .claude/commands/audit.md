---
description: Deep security audit of a feature or area — finds vulnerabilities in existing code, not just recent changes
argument-hint: [area to audit, e.g. "auth flow" or "API routes"]
allowed-tools: Read, Grep, Glob, Bash(git log:*)
---

Security audit of: $ARGUMENTS (if empty, audit the most security-sensitive area: auth, data access, API surface)

Locate the relevant code with Grep/Glob first (auth handlers, DB access, API routes, env usage), then assess against these risks. For each finding give severity (critical/high/medium/low), file:line, the concrete attack, and the fix:

1. **Injection** — SQL/NoSQL (even via ORM with raw fragments), command injection, XSS (unescaped output reaching the DOM)
2. **Broken auth & authorization** — missing session checks; authentication present but authorization missing (can user A access user B's resource? IDOR); privilege escalation
3. **Secrets exposure** — keys/credentials in code, logs, error messages, client bundles, or git history
4. **Sensitive data** — PII/financial data sent to client unnecessarily, missing encryption at rest where it matters, leaky error messages
5. **Input/output handling** — unvalidated inputs, missing Zod/schema validation at trust boundaries, mass-assignment, unsafe deserialization
6. **Rate limiting & abuse** — auth endpoints, password reset, write/expensive operations without throttling
7. **Dependencies** — known-vulnerable packages (flag for manual check; do not run installs)
8. **Config** — permissive CORS, missing security headers, debug mode reachable in prod, default credentials

Output: findings grouped by severity, critical first. End with a one-line verdict: SAFE TO SHIP / FIX BEFORE SHIP / NEEDS REVIEW.
Do not modify any files — this is read-only assessment. Do not run package installs or network calls.
