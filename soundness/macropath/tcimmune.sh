#!/usr/bin/env bash
# tcimmune.sh — regenerate baretc::GLOB_IMMUNE_STD / GLOB_IMMUNE_CORE by EXECUTION (SOUNDNESS R1908/R1914, lane rust-macropath-l3).
#   bash soundness/macropath/tcimmune.sh [edition]      # default 2021; prints both lists
# For each toolchain macro name n: a dependency exports `n!`, the crate does `use dep::*;` and invokes `n!()` bare. E0659 = the glob
# cannot rebind n ("immune"); a clean build = it can. Two crate kinds: std, and `#![no_std]`. Names that fail for another reason
# (try: deprecated) are never listed, i.e. they stay rebindable (the hold direction).
set -u
ed=${1:-2021}
names="println print eprintln eprint format format_args write writeln panic assert assert_eq assert_ne debug_assert debug_assert_eq debug_assert_ne vec unreachable unimplemented todo matches dbg concat stringify line column file module_path env option_env include_str include_bytes include cfg compile_error thread_local try ready addr_of addr_of_mut offset_of assert_matches debug_assert_matches const_format_args format_args_nl concat_idents trace_macros log_syntax asm global_asm naked_asm pin is_x86_feature_detected is_aarch64_feature_detected is_arm_feature_detected unreachable_unchecked"
T=$(mktemp -d "${TMPDIR:-/tmp}/tcimmune.XXXXXX")
std=""; nostd=""
for n in $names; do
  mkdir -p "$T/$n"
  # shellcheck disable=SC2016
  printf '#[macro_export] macro_rules! %s { ($($t:tt)*) => { } }\n' "$n" > "$T/$n/dep.rs"
  rustc --edition "$ed" --crate-type lib --crate-name "dep_$n" "$T/$n/dep.rs" -o "$T/$n/libdep.rlib" 2>/dev/null || continue
  for kind in std nostd; do
    pre=""; [ $kind = nostd ] && pre="#![no_std]"
    if [ "$ed" = 2015 ]; then use="extern crate dep_$n; use dep_$n::*;"; else use="use dep_$n::*;"; fi
    printf '%s\n%s\npub fn f() { %s!(); }\n' "$pre" "$use" "$n" > "$T/$n/$kind.rs"
    out=$(rustc --edition "$ed" --crate-type lib --extern "dep_$n=$T/$n/libdep.rlib" "$T/$n/$kind.rs" -o "$T/$n/$kind.rlib" 2>&1)
    if echo "$out" | grep -q E0659; then [ $kind = std ] && std="$std $n" || nostd="$nostd $n"; fi
  done
done
echo "GLOB_IMMUNE_STD:  $(echo "$std" | tr ' ' '\n' | sort | tr '\n' ' ')"
echo "GLOB_IMMUNE_CORE: $(echo "$nostd" | tr ' ' '\n' | sort | tr '\n' ' ')"
rm -rf "${T:?}"
