# quotes

A small program that computes price quotes for a fixed catalogue.

## Modules

- `money` — value types (`Money`, `Currency`) and arithmetic on amounts.
- `pricing` — computes a quote from the catalogue and an FX rate. The rate is
  held in `Pricing` and can be updated with `set_rate`.

## Running

`cargo run` prints one quote per SKU.
