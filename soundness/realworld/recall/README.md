# Recall batteries for the real-world oracle

Two different questions, two batteries. Neither is about candor's precision; both are about whether a
**green** result means anything.

## 1. `recall.sh` — non-syscall recall (what strace cannot see)

`strace` distinguishes Net/Fs/Exec. It cannot distinguish Db, Log, Rand, Env or Clock from ordinary
register and fd traffic, so for those the ground truth is documented crate semantics rather than the kernel
(`expected.json`). Syntactic, so it needs no Linux, no strace and no dep builds — it runs anywhere.

## 2. `disclosure_recall.sh` — is the syscall oracle able to fail?

The syscall arm (`../run.sh`) reports **0 violations**. That is only evidence the honesty invariant held if
the oracle *could* have reported one. A marker that never fires, a report path that resolves to nothing, or
a verdict that accepts any non-empty claim would each produce a silent, permanent green — indistinguishable
from soundness, and far more likely than soundness.

So we seed the exact defect the oracle exists to catch:

| pass | what is falsified | required outcome |
|---|---|---|
| `control` | nothing | green — you cannot calibrate against a broken oracle |
| `silent` | every effect **and** every disclosure flag stripped | red on every falsifiable driver |
| `wrong` | a decoy effect (`Rand`) replaces the real one, disclosure stripped | red on every falsifiable driver, **plus** a fabrication on the pure control |
| `transitive` | **only the entry point**, with the effect's leaf left honest | red on every falsifiable driver *(per-function oracle only)* |

`silent` is the canonical false all-clear: the program still issues the syscalls, the signature now claims
sound-complete purity. `wrong` separates *"the verdict checks for **the** effect"* from *"the verdict checks
for **an** effect"* — a checker that only tested non-emptiness passes `silent` and fails only here.

`transitive` is the sharp one, and it applies to the per-function oracle (`../pf/run_pf.sh`) alone. Both of
the others falsify every frame at once, so passing them is equally consistent with an oracle that only
adjudicates the frame where the effect was *issued*. This mutant confines the lie to a single transitive
caller — which is exactly the shape of a dropped call-graph edge, the (A3) failure — and requires the
verdict to catch it there. A program-level verdict cannot see it at all, since the effect still appears
somewhere in the report; that asymmetry is the point.

The falsification is injected into candor's report **downstream of the analyzer and upstream of the
verdict** (`CANDOR_ORACLE_MUTATE`, unset in every normal run), which is precisely where a false all-clear
lives. Nothing else is stubbed: each pass builds the drivers, executes them, and reads real traces, so what
gets calibrated is the deployed instrument end to end rather than a re-implementation of its verdict.

The hook is safe to ship because every mutation is **monotone in the red direction**: each one removes
effects or removes disclosure, so it can only add violations, never remove one. There is no setting of
`CANDOR_ORACLE_MUTATE` that turns a genuine finding green — it is not a suppression switch.

### Reading the result

Two numbers, and they must travel together:

- **recall on the falsifiable set** — of the drivers whose effect demonstrably executed this run, how many
  seeded lies were caught.
- **the uncalibrated remainder** — drivers that did not build, or whose effect did not execute, printed by
  name. No evidence either way for these.

An oracle falsifiable on 3 of 20 drivers has a recall of 1.0 and is still nearly blind. Reporting the first
number alone would be exactly the kind of flattering summary this project treats as a defect.

### Scope

Only Net/Fs/Exec are syscall-visible under the harness's filter; Env and Clock resolve in the vDSO and libc
and never appear. Disclosure recall is therefore measured on the syscall-visible subset, and says nothing
about the arms' blindness to the rest — that is what battery 1 exists for.
