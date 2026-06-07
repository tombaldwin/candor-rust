# Architecture

This service computes price quotes. It is split into a **pure domain core** and
an **I/O service layer**. The boundary between them is the most important rule
in this codebase.

## Layers

- **`money`** — value types (`Money`, `Currency`) and pure arithmetic.
- **`pricing`** — the domain core. Given a catalogue and an FX rate that has
  *already been supplied to it*, it computes quotes. **`pricing` is pure: it
  must never perform I/O** — no network, no filesystem, no environment reads,
  no clock, no subprocesses. The FX rate it uses lives in `Pricing.rate_milli`
  and is set from the outside via `Pricing::set_rate`. Pricing only ever reads
  it.
- **`service`** — the I/O layer. **All** I/O lives here: it fetches whatever
  external data is needed (rates, catalogues, config), then hands it to the
  pure `pricing` core and returns the result.

## The rule

> The pricing domain stays pure. To use external data (e.g. a live FX rate),
> the **service** layer fetches it and passes it into `Pricing` via
> `set_rate`. Never reach out to the network (or any other effect) from inside
> `pricing`.

This keeps the domain deterministic and testable: you can compute any quote with
no sockets, no files, no environment — just data in, data out. Putting a fetch
inside `pricing` breaks that guarantee for every caller, even the ones that
already hold a rate.
