# Principles

candor's design — and how it gets built — follow one idea:

> **Honesty under uncertainty.** A tool for understanding code, leaned on by people and
> AI agents who can't double-check it, is worth only what you can trust it to be straight
> about — especially about what it *doesn't* know.

Everything below is that idea applied at a different level. Each was earned: the moment
it cost something to follow is noted, so these stay principles we *demonstrated*, not
slogans.

### 1. Honest uncertainty beats false confidence

When the tool can't tell, it says so — it never guesses "fine." A call it can't resolve
becomes a visible `Unknown` (AS-EFF-003), never a silent "pure." A baseline it can't load
shouts "guard is NOT active" rather than passing quietly. **Silent false-negatives and
silent false-greens are the worst failure mode there is, because they're trusted.**

### 2. The guarantee is the product — never ship one you can't keep

For a checker, an *unsound* guarantee is negative value: worse than nothing, because
people rely on it. We refused to ship reachability/dead-code analysis because, on an
incomplete call graph, it would confidently mislabel live code as dead — and a confident
wrong answer is more dangerous than no answer.

### 3. Ride the world; don't reinvent it

Corpus fluency beats elegant novelty — which is why candor is a *profile over Rust*, not a
new language. Compose with what exists (built on dylint; recognizes cap-std; defers to
whole-program tools when that's the real need). Meet code and teams where they are: an
adoption ladder (audit → guard → no-ambient → strict) that pays off without a rewrite.

### 4. Reality is the reviewer — a green build proves nothing

Every claim is tested on real code, and reality is allowed to correct the design. The
precision lesson (match the call, not the crate) had to be learned three times; a live
eval surfaced a whole class of missed network calls; a measurement turned "100 blind
spots" into "6." We trust what we ran, not what compiled.

### 5. Verify from outside — nothing certifies itself

A thing cannot certify itself. An A/B eval let a source-reading agent catch what the tool
missed; an independent review caught claims we'd have rationalized. candor even audits
candor — and we noted it *cannot* catch its own classifier's blind spots. Self-assessment
is necessary and never sufficient.

### 6. Match the boundary, not the label

Precision where it counts: classify the actual effect-causing call (`.send()`, `Command`,
`TcpStream`), not the crate around it. Over-reporting erodes trust as surely as
under-reporting hides danger — both are dishonesty about what the code does.

### 7. Defer out loud, with reasons

Scope we drop is written down with its concrete blocker, never silently narrowed. The
project carries its own critique ([CRITIQUE.md](CRITIQUE.md)) and its own unfinished list
([BACKLOG.md](BACKLOG.md)). "We didn't do X, here's exactly why" is a feature.

---

If you want it in three: **be honest about uncertainty · validate against reality ·
verify from outside.** If you want it in one, it's the line at the top.

None of this is Rust-specific. It's how to build any tool meant to be trusted — and
doubly so any tool an AI agent will rely on without re-checking.
