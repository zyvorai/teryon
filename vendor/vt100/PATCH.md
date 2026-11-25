# Vendored vt100 0.18.1 (patched)

Upstream: https://crates.io/crates/vt100/0.18.1 (MIT, license unchanged). Dev-dependencies were removed from
the vendored `Cargo.toml`; tests and examples are not shipped in the published crate.

Patches (each marked `TERYON patch` or kept in `src/links.rs`):

1. Stable line numbers: `Grid` counts rows that scroll into the scrollback (`scrolled_total`, exposed as
   `Screen::scrolled_total()`). A resize that pushes rows into the scrollback counts them; one that pulls rows
   back onto the screen subtracts them. `CSI 3 J` clears the scrollback without resetting the counter.
2. OSC 8 hyperlinks: `src/links.rs`, a link id in the cell attributes (not emitted in formatted output),
   `Screen::hyperlinks()`.
3. `Screen::erase_count()` (CSI 2 J and RIS), used to drop inline images when the screen is cleared.
4. `Screen::advance_rows()`, used to leave room for an inline image.

Remove this directory and the `[patch.crates-io]` entry if upstream gains equivalents.
