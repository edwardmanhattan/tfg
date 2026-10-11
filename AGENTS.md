## Agent skills

### Issue tracker

Issues live in GitHub Issues (via `gh` CLI). See `docs/agents/issue-tracker.md`.

### Triage labels

Default five canonical labels, strings equal to role names. See `docs/agents/triage-labels.md`.

### Build / compile

Do not compile just to check that a change parses. The user compiles the code
themselves on their own time. Make the change and report it.

Exceptions:

- **GPUI migration in progress.** While the migration runs, the agent may
  compile and test to verify its own slices (`cargo test --lib`,
  `cargo check`, proto-crate builds). A UI surface still needs a driven
  binary and a capture before it is reported as working.
- **Fixing a UI defect.** A defect in this console is invisible to the compiler
  and to the test suite. Drive the real binary and look at it. See
  `docs/ui-verify-handoff.md`, and `bash scripts/ui-shoot/env-up.sh` for the
  environment.
- **Before reporting a change to a UI surface as working.** "It compiles" is not
  evidence about a window.

### Domain docs

Single-context layout (root `CONTEXT.md` + `docs/adr/`). See `docs/agents/domain.md`.
