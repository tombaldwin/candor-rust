# Ground truth — `bottom`, `DataStore::get_data` (Fs)

Established INDEPENDENTLY of candor and frozen before the trial matrix. Machine-readable list:
[bottom-groundtruth.txt](bottom-groundtruth.txt) (26 functions).

- **Effect gained:** `Fs`. **Probed function:** `app::data::store::DataStore::get_data` — genuinely pure
  today (returns `&self.cached StoredData`). Natural gain: read live system data from the OS per call.
- **Question:** if `get_data` performed `Fs`, which other functions in the library crate would
  transitively perform it — i.e. the complete set of transitive callers.

## Method (anti-circularity)

Two independent strong-model source-only tracers + a hand grep of the 16 direct call sites; every
disagreement resolved against source. candor's `callers` output is reported as a *finding*, not the key.

## The adjudicated set — 26 functions

See [bottom-groundtruth.txt](bottom-groundtruth.txt). Two fan-in clusters: the **draw pipeline**
(`canvas::Painter::draw_data` → `draw_widgets_with_constraints` / `draw_cpu` / `draw_network` → the 10
per-widget `draw_*` methods) and the **input/update path** (`event::handle_key_event_or_break` /
`handle_mouse_event` → the `App` key/mouse handlers + the `handle_char`→`on_char_key`→`on_space_key`
chain + `App::update_data`), both converging on `try_drawing` / `start_bottom`. The bin entry `fn main`
(the sole caller of `start_bottom`) is out of the scanned library and excluded.

## Adjudication log

- Tracers + candor converged on the 26-function core. Tracer B missed `on_right_key` (a verified direct
  caller candor + tracer A both have); included. Both tracers listed `main` (the bin entry) — excluded
  as out-of-library-scope (it is not in the scanned lib report); treated as don't-care in grading.
- Key disambiguations all three handled: `DataCollector::update_data` (≠ `App::update_data`) and
  `ProcessKillDialog::on_left_key/on_right_key` (≠ `App`'s) are same-named methods on other types that
  do NOT reach `get_data` — excluded.

## Candor's result (finding, not the key)

`candor-query callers <report> DataStore::get_data 1` returned exactly these 26 — **recall 26/26**
against the adjudicated truth, matching tracer A and catching the caller tracer B missed. So despite the
cosmetic shutdown ICE, candor's report is complete for this tree. (Easy-end note: this tree is
greppable — 16 direct callers — so it is far easier to trace by hand than delta's 61-fn deep tree; see
the cross-target comparison in ../../RESULTS-realworld.md.)
