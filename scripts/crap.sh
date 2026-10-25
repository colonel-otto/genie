#!/usr/bin/env bash
# The CRAP gate (`.cargo-crap.toml`): the workspace's tests under coverage (cargo-llvm-cov over
# cargo-nextest, LCOV into coverage/lcov.info, which git ignores), then cargo-crap scores every
# function. The pre-commit hook runs the gate.
#
#     scripts/crap.sh [--gate]          fails on a function above the threshold that the debt
#                                       list (.config/crap-debt.json) does not hold at that score
#                                       or higher; without a debt list, on any function above it
#     scripts/crap.sh --report [ARGS…]  cargo-crap's table (its ARGS, e.g. --top 40), no gate
#     scripts/crap.sh --bless           the debt list becomes this run's functions above the
#                                       threshold; with none, the list goes and the gate is
#                                       absolute
#
# CRAP_COVERAGE=keep scores coverage/lcov.info as it is, without running the tests again: right
# only while no source changed since that run (a new function's lines are not in it).
# Needs cargo-llvm-cov, cargo-nextest, cargo-crap, jq and LLVM tools of rustc's LLVM version:
# `rustup component add llvm-tools-preview`, else another installed toolchain's tools of the
# same LLVM major version are taken, or LLVM_COV and LLVM_PROFDATA name them.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
mode=${1:---gate}
[ $# -gt 0 ] && shift
debt=.config/crap-debt.json
lcov=coverage/lcov.info
threshold=$(sed -n 's/^threshold *= *//p' .cargo-crap.toml)

# cargo-llvm-cov's LLVM tools: the toolchain's own, else those of another installed toolchain
# whose LLVM has rustc's major version (profiles stay readable within one major version)
llvm_tools() {
    if [ -n "${LLVM_COV:-}" ] && [ -n "${LLVM_PROFDATA:-}" ]; then
        return
    fi
    local host major own tc bin
    host=$(rustc -vV | sed -n 's/^host: //p')
    major=$(rustc -vV | sed -n 's/^LLVM version: \([0-9]*\).*/\1/p')
    own=$(rustc --print sysroot)/lib/rustlib/$host/bin
    if [ -x "$own/llvm-cov" ] && [ -x "$own/llvm-profdata" ]; then
        return
    fi
    if command -v rustup > /dev/null; then
        for tc in $(rustup toolchain list | cut -d' ' -f1); do
            bin=$(rustc +"$tc" --print sysroot)/lib/rustlib/$host/bin
            if [ -x "$bin/llvm-profdata" ] &&
                "$bin/llvm-cov" --version 2> /dev/null | grep -q "LLVM version $major\."; then
                export LLVM_COV=$bin/llvm-cov LLVM_PROFDATA=$bin/llvm-profdata
                return
            fi
        done
    fi
    echo "crap: no LLVM tools of LLVM $major: rustup component add llvm-tools-preview" >&2
    exit 2
}

if [ "${CRAP_COVERAGE:-}" != keep ] || [ ! -f "$lcov" ]; then
    llvm_tools
    mkdir -p coverage
    # the run's output goes to a log, shown only when it fails
    if ! cargo llvm-cov nextest --workspace --lcov --output-path "$lcov" \
        > coverage/tests.log 2>&1; then
        tail -n 60 coverage/tests.log >&2
        echo "crap: the tests under coverage failed (coverage/tests.log)" >&2
        exit 1
    fi
fi

case $mode in
--report)
    exec cargo crap --lcov "$lcov" --workspace "$@"
    ;;
--bless)
    # by file, function and line: the committed list changes only where a function changed
    cargo crap --lcov "$lcov" --workspace --format json --sort file --output coverage/crap.json
    jq --argjson t "$threshold" \
        '{version, entries: [.entries[] | select(.crap > $t) | del(.uncovered)]}' \
        coverage/crap.json > coverage/crap-debt.json
    n=$(jq '.entries | length' coverage/crap-debt.json)
    if [ "$n" = 0 ]; then
        rm -f "$debt"
        echo "crap: no function above $threshold: the debt list is gone, the gate is absolute"
    else
        cp coverage/crap-debt.json "$debt"
        echo "crap: $n functions above $threshold in $debt"
    fi
    ;;
--gate)
    if [ ! -f "$debt" ]; then
        exec cargo crap --lcov "$lcov" --workspace --summary --fail-above
    fi
    cargo crap --lcov "$lcov" --workspace --baseline "$debt" --format json \
        --output coverage/crap-delta.json
    jq -r --argjson t "$threshold" '
        .entries[]
        | select(.crap > $t and (.baseline_crap == null or .crap > .baseline_crap + 0.01))
        | "\(.file):\(.line) \(.function): CRAP \(.crap | floor), complexity \(.cyclomatic),"
          + " \(.coverage // 0 | floor)% covered, "
          + (if .baseline_crap == null then "new above \($t)"
             else "was \(.baseline_crap | floor)" end)' \
        coverage/crap-delta.json > coverage/crap-gate.txt
    if [ -s coverage/crap-gate.txt ]; then
        cat coverage/crap-gate.txt >&2
        echo "crap: give them tests or split them into named functions" >&2
        exit 1
    fi
    echo "crap: no function rose above $threshold ($(jq '.entries | length' "$debt") in the debt list)"
    ;;
*)
    echo "crap: unknown mode $mode (--gate, --report, --bless)" >&2
    exit 2
    ;;
esac
