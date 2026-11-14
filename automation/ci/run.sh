#!/bin/bash
# CI checks, run inside the image from automation/ci/Dockerfile. Mirrors scripts/check.ps1
# and adds the Windows build that end-to-end tests use.
set -euo pipefail
cd "$(dirname "$0")/../.."

step() { printf '\n=== %s ===\n' "$1"; }

step "UTF-8"
python3 automation/ci/check_utf8.py

step "Format"
cargo fmt --all -- --check

step "Clippy"
cargo clippy --workspace --all-targets --all-features -- -D warnings

step "Tests with coverage"
cargo llvm-cov --workspace --all-features --fail-under-lines 95

step "Windows build"
cargo build --release --target x86_64-pc-windows-gnu -p staminator-desktop
mkdir -p ci-out
cp "${CARGO_TARGET_DIR:-target}"/x86_64-pc-windows-gnu/release/*.exe ci-out/
ls -l ci-out
