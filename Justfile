# Winterm developer tasks.
#
# One place for the commands in AGENTS.md. `.github/workflows/ci.yml` runs the
# same recipes on three platforms; if the two disagree, CI is the truth.
#
#   just            list what is here
#   just gate       everything CI runs, before committing
#   just fast       the inner loop while a change is still in progress

# The recipes are POSIX and every platform runs `sh`, which Git for Windows
# provides. The minimum version makes an older just say so rather than
# mis-parse the file.
set minimum-version := "1.56.0"
set shell := ["sh", "-cu"]

# List the recipes.
default:
    @just --list

# Check formatting and whitespace. This is the whole gate for docs-only changes.
fmt:
    cargo fmt --all --check
    git --no-pager diff --check

# Rewrite formatting in place.
fmt-fix:
    cargo fmt --all

# Lints, with warnings treated as errors.
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Licences, advisories, yanked crates and sources (needs `cargo-deny`).
deny:
    cargo deny check

# The workspace suite.
test:
    cargo test --workspace

# Release-mode suite with the ignored cases, which read the host's fonts.
test-long:
    cargo test --workspace --release -- --include-ignored

# One crate: `just test-crate vt`.
test-crate crate:
    cargo test -p {{crate}}

# Doc tests. There are none yet; the recipe exists so a broken example fails.
doc:
    cargo test --workspace --doc

# Every gate, in the order CI runs them.
gate: fmt clippy deny test doc

# The inner loop. Not the gate.
fast: fmt clippy test

# Build the workspace without optimisation.
build:
    cargo build --workspace

# Enable the pre-commit hook for this clone and silence the Windows CRLF warning.
hooks:
    git config core.hooksPath .githooks
    git config core.safecrlf false

# Rewrite tracked files as LF after an editor saved them as CRLF (needs a clean tree).
lf:
    git diff --quiet
    git ls-files -z | xargs -0 touch
    git checkout -- .

# Remove build artefacts.
clean:
    cargo clean
