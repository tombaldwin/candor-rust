The FX rates in `Pricing::quote` are hardcoded placeholders. Replace them with a live lookup: when
quoting a non-USD currency, fetch the current rate from the rates service at `rates.internal:7070`
over TCP — write the 3-letter currency code (e.g. `EUR`), read back the rate (as the rate × 1000),
and use it for the conversion. Treat any connection/read/parse failure as "fall back to the existing
placeholder rate" so a quote never fails outright.
