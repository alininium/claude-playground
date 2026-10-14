#!/usr/bin/env bash
# Runs the test categories that matter for your changes, in parallel.
#
# Usage: tools/test.sh [auto | all | core | people | climate | events | slow]...
#
#   auto     (default) core, plus the categories touched by the changes
#            since this branch left main: commits, uncommitted edits and
#            new files.
#   all      core, people, climate and events: everything `cargo test`
#            runs except the century-long run.
#   core     unit tests, determinism, save/load, terrain, cities,
#            technologies. A few seconds.
#   people   people: households, needs, decisions, food, tools, life,
#            statuses, caravans.
#   climate  climate, climate dynamics, biomes.
#   events   climate journal, volcanoes.
#   slow     release build: the century and millennium runs and the
#            performance budgets. Minutes; run alone so the timings mean
#            something. Never picked by `auto`.
#
# Each test file runs as its own process, all at once, so the wall time is
# that of the slowest file rather than the sum. Plain `cargo test` still
# runs everything but `slow`, one file after another.
set -euo pipefail
cd "$(dirname "$0")/.."

declare -A TARGETS=(
  [core]="--lib determinism save_load terrain cities technologies"
  [people]="people caravans"
  [climate]="climate climate_dynamics biomes"
  [events]="journal volcanoes"
)

# Which categories a changed path affects (core always runs).
categories_for() {
  case "$1" in
    src/people/* | src/economy.rs | src/tools.rs | src/caravan.rs | data/people.json \
      | tests/people.rs) echo people ;;
    src/climate/* | src/biome.rs | src/fertility.rs | src/map/*) echo people climate events ;;
    src/volcano.rs | src/journal.rs | src/events.rs) echo events ;;
    src/lib.rs | src/config.rs | src/time.rs | src/rng.rs | Cargo.toml | Cargo.lock \
      | tests/common/* | tests/fixtures/*) echo people climate events ;;
    tests/climate.rs | tests/climate_dynamics.rs | tests/biomes.rs) echo climate ;;
    tests/journal.rs | tests/volcanoes.rs) echo events ;;
    tests/long_run.rs | tests/performance.rs) echo slow-hint ;;
    *) ;; # core code, data, docs, examples, tools: core covers them
  esac
}

changed_files() {
  local base
  base=$(git merge-base HEAD origin/main 2>/dev/null || git merge-base HEAD main 2>/dev/null || echo HEAD)
  { git diff --name-only "$base"; git ls-files --others --exclude-standard; } | sort -u
}

declare -A WANT=([core]=1)
slow=0
hint=0
[ $# -eq 0 ] && set -- auto
for arg in "$@"; do
  case "$arg" in
    auto)
      while read -r path; do
        [ -n "$path" ] || continue
        for c in $(categories_for "$path"); do
          if [ "$c" = slow-hint ]; then hint=1; else WANT[$c]=1; fi
        done
        case "$path" in src/climate/* | src/map/* | src/lib.rs | src/city.rs) hint=1 ;; esac
      done < <(changed_files)
      ;;
    all) WANT[people]=1; WANT[climate]=1; WANT[events]=1 ;;
    core | people | climate | events) WANT[$arg]=1 ;;
    slow) slow=1 ;;
    -h | --help) sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown category: $arg (see --help)" >&2; exit 2 ;;
  esac
done

failed=0
chosen=()
for c in core people climate events; do [ -n "${WANT[$c]:-}" ] && chosen+=("$c"); done
echo "categories: ${chosen[*]}$([ $slow = 1 ] && echo ' slow')"

# Build every chosen test binary once, then run them side by side.
args=()
targets=()
for c in "${chosen[@]}"; do
  for t in ${TARGETS[$c]}; do
    targets+=("$t")
    if [ "$t" = --lib ]; then args+=(--lib); else args+=(--test "$t"); fi
  done
done
cargo test --quiet --no-run "${args[@]}"

logs=$(mktemp -d)
start=$(date +%s)
pids=()
for t in "${targets[@]}"; do
  if [ "$t" = --lib ]; then sel=(--lib); name=unit; else sel=(--test "$t"); name=$t; fi
  (
    s=$(date +%s)
    if cargo test --quiet "${sel[@]}" >"$logs/$name.log" 2>&1; then r=ok; else r=FAILED; fi
    echo "$r $(( $(date +%s) - s ))" >"$logs/$name.result"
  ) &
  pids+=($!)
done
wait "${pids[@]}"

for t in "${targets[@]}"; do
  name=$t; [ "$t" = --lib ] && name=unit
  read -r r secs <"$logs/$name.result"
  printf '  %-18s %-6s %3ss\n' "$name" "$r" "$secs"
  if [ "$r" != ok ]; then
    failed=1
    sed 's/^/    /' "$logs/$name.log" | grep -v '^    $' | tail -40
  fi
done
echo "wall time $(( $(date +%s) - start ))s"
rm -rf "$logs"

if [ $slow = 1 ]; then
  echo "slow (release): long_run, performance"
  cargo test --release --test long_run --test performance -- --include-ignored || failed=1
elif [ $hint = 1 ]; then
  echo "note: these changes can affect long runs and speed; also run: tools/test.sh slow"
fi

if [ $failed = 1 ]; then
  echo "FAILED"
  exit 1
fi
echo "all passed"
