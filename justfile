# https://github.com/casey/just

[private]
default:
    @just --list

build:
    cargo build --release

build-all:
    cargo build --release --all-features

run *args:
    cargo run --all-features -- {{ args }}

fmt:
    cargo fmt
    cargo clippy --all-targets --all-features -- -D warnings
    @ # cargo shear --fix # cargo install shear

check:
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings

test: fmt
    cargo test --all-targets --all-features

install-hook:
    @printf '#!/bin/sh\nset -e\njust check\n' > .git/hooks/pre-commit
    @chmod +x .git/hooks/pre-commit

remove-hook:
    @rm .git/hooks/pre-commit

add-tag:
    #!/usr/bin/env bash
    set -euo pipefail
    VERSION=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
    git push origin main
    git tag -a "v${VERSION}" -m "Release v${VERSION}"
    git push origin "v${VERSION}"

# `just remove-tag v0.0.0` or `just remove-tag` (uses fzf)
remove-tag VERSION="":
    #!/usr/bin/env bash
    set -euo pipefail
    tag="{{ VERSION }}"
    [ -z "$tag" ] && tag=$(git tag | sort -V | fzf --prompt="Select tag to remove: ")
    [ -z "$tag" ] && echo "No tag selected" && exit 1
    git tag -d "$tag"
    git push --delete origin "$tag"

# Undo last commit locally and on remote, keeping changes staged.
undo-commit:
    #!/usr/bin/env bash
    set -euo pipefail

    upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}')
    remote="${upstream%%/*}"
    branch="${upstream#*/}"

    git reset --soft HEAD~1
    git push --force-with-lease "$remote" "HEAD:$branch"
