#!/bin/bash
set -e

echo "Running tests with coverage and enforcing 80% minimum line coverage..."

cargo llvm-cov --fail-under-lines 80 --lcov --output-path lcov.info

echo "Coverage check passed! (>= 80% line coverage)"
