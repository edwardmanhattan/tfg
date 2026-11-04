#!/usr/bin/env bash
# Where the built binary is — SOURCED, never executed.
#
# The runner scripts used `cargo run`, which rebuilds the tree behind the
# operator's back, picks a profile for them, and hides which binary it ran.
# The build is the operator's now: `cargo build --bin <name>` makes it once and
# `target/` keeps it. This is the one answer to "which file is that", so four
# scripts cannot drift into four spellings of it.
#
# Resolution order:
#
#   1. `TFG_BIN_DIR` — the operator naming a directory outright (a release
#      build, a worktree's target, a bundle). It wins ALONE: a directory that
#      was named and does not hold the binary refuses, rather than quietly
#      running one from somewhere else.
#   2. this checkout's `target/debug` — what `cargo build --bin <name>` writes,
#      and what `cargo run` would have run.
#   3. this checkout's `target/release` — the smaller, faster one, if that is
#      the build that exists.
#
# A checkout's own target dir matters here: `.cargo/config.toml` gives every
# checkout a target beside itself, so an absolute path in a script would run
# ANOTHER checkout's binary — the capture-out-of-date failure that file was
# written to stop.
tfg_bin() {
    local name="$1"
    local root
    root="${TFG_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)}"
    local candidate
    if [ -n "${TFG_BIN_DIR:-}" ]; then
        candidate="${TFG_BIN_DIR}/${name}"
        if [ -x "$candidate" ]; then
            printf '%s\n' "$candidate"
            return 0
        fi
        echo "REFUSING: ${candidate} is not built. Run: cargo build --bin ${name}" >&2
        return 1
    fi
    for candidate in "${root}/target/debug/${name}" "${root}/target/release/${name}"; do
        if [ -x "$candidate" ]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done
    echo "REFUSING: ${name} is not built. Run: cargo build --bin ${name}" >&2
    return 1
}
