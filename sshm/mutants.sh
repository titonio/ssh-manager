#!/bin/bash
# Mutation-test this crate with cargo-mutants.
#
# Coverage says which lines a test run touched; a mutant says whether anything
# actually *depends* on the line: each one replaces a function body or a
# condition with a trivial value, and a mutant that no test notices is a
# behaviour nobody is guarding. It is the gate that catches an assertion written
# backwards, which a 100%-covered line hides.
#
# Options that can live in a file are in .cargo/mutants.toml; this script adds
# the one that cannot.
#
#   ./mutants.sh              # every mutant in the crate
#   ./mutants.sh --in-diff d  # only mutants in the lines `d`'s diff touched
#
# Results, including a log per mutant, land in mutants.out/ — missed.txt is the
# list to work through, and `less mutants.out/out.txt` shows the whole run.
set -e

cd "$(dirname "$0")"

# --in-place mutates the files in this directory instead of copying the tree.
# Copying is the default, and here it is ruinous: the copy does not reuse
# `target/`, so every mutant rebuilds ratatui, reqwest and the rest from zero.
# In place, a mutant costs one `sshm` rebuild and one run of the whole suite:
# about 10s all in, so the full sweep is a couple of hours rather than a week.
#
# It also means the run edits the working tree while it goes: do not edit, commit
# or `cargo test` in this checkout until it finishes, and re-check `git diff` if
# it is interrupted — cargo-mutants restores each file, but not after a kill.
exec cargo mutants --in-place "$@"
