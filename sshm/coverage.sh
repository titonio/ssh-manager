#!/bin/bash
set -e

echo "Running tests with coverage and enforcing 80% minimum line coverage..."

# --all-features matches the CI coverage run, so the CRAP gate below measures
# the same thing here that it does there.
cargo llvm-cov --all-features --fail-under-lines 80 --lcov --output-path lcov.info

echo "Coverage check passed! (>= 80% line coverage)"

# CRAP gate: flags functions that are complex *and* untested. Threshold and
# --fail-above come from .cargo-crap.toml; run from this directory so the
# default tests/** excludes match.
echo "Running CRAP gate..."
cargo crap --lcov lcov.info
echo "CRAP gate passed! (no function above threshold 12)"
