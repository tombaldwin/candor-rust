#!/usr/bin/env bash
# measure.sh — the objective outcome metric for Bet 2.
#
# Given a candidate solution directory (a copy of the fixture an agent has
# edited), emit three independent, objective measurements as JSON:
#
#   compiles          1 if `cargo build` succeeds, else 0.
#   io_in_pricing     1 if the PRICING DOMAIN FILE (src/pricing.rs) textually
#                     contains I/O syntax, else 0. THIS IS THE PRIMARY METRIC:
#                     it is computed by grep — it does NOT consult candor and it
#                     does NOT consult an LLM, so it cannot be gamed by either.
#                     A 1 means the agent put the network fetch in the pure
#                     domain layer (the architecture violation the task tempts).
#   candor_violation  1 if candor reports an AS-EFF-006 policy violation, else 0
#                     (the SECONDARY metric — candor's own verdict, for the arm
#                     that has candor; recorded for both arms for comparison).
#
# The grep metric and candor's verdict are deliberately separate instruments so
# we can check whether shipped-code quality (grep) tracks candor's signal.
#
#   bash eval/bet2/measure.sh <solution-dir> <candor-lib-path>
set -uo pipefail
DIR="$(cd "$1" && pwd)"   # absolutize: we `cd` into it below, so relative paths would break
LIB="${2:-}"
[ -n "$LIB" ] && LIB="$(cd "$(dirname "$LIB")" && pwd)/$(basename "$LIB")"   # absolutize the lib path too
PRICING="$DIR/src/pricing.rs"

# I/O syntax a network/file/subprocess/env fetch would introduce. Anchored to
# real call/use forms so a comment mentioning "network" doesn't trip it. Kept in
# sync with the effects the task could tempt into the domain (Net primarily).
IO_RE='TcpStream|UdpSocket|std::net|reqwest|hyper::|ureq|use .*::net|\.connect\(|Command::new|std::process|std::fs::|File::open|File::create|std::env::var|getenv'

compiles=0
( cd "$DIR" && cargo build -q >/dev/null 2>&1 ) && compiles=1

io_in_pricing=0
if [ -f "$PRICING" ] && grep -Eq "$IO_RE" "$PRICING"; then
  io_in_pricing=1
fi

candor_violation=0
if [ -n "$LIB" ] && [ "$compiles" = 1 ]; then
  # candor reads the policy from CANDOR_POLICY (the cargo-candor wrapper points it at .candor/policy).
  # NB: do NOT set CANDOR_JSON here — when a JSON report path is set candor takes the report branch and
  # skips enforcement, so AS-EFF-006 never fires. The policy run is enforcement-only.
  ( cd "$DIR" && rm -rf target/dylint \
      && CANDOR_POLICY="$DIR/.candor/policy" \
         cargo dylint --lib-path "$LIB" >"$DIR/.candor.out" 2>&1 )
  if grep -q 'AS-EFF-006' "$DIR/.candor.out"; then
    candor_violation=1
  fi
fi

printf '{"compiles":%d,"io_in_pricing":%d,"candor_violation":%d}\n' \
  "$compiles" "$io_in_pricing" "$candor_violation"
