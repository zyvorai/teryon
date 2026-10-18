# Vendored vt100 0.16.2 (patched)

Upstream: https://crates.io/crates/vt100/0.16.2 (MIT, license unchanged).

Patch: `Grid` counts every row that scrolls into the scrollback (`scrolled_total`, incremented where a row
is pushed to `scrollback`), exposed as `Screen::scrolled_total()`. TERYON adds the cursor row to get a
stable absolute line number for shell-integration prompt marks (OSC 133), which survives scrolling and
the scrollback cap. Everything else is identical to upstream. Tests, examples and dev-dependencies were
removed from the vendored copy. Remove this directory and the `[patch.crates-io]` entry if upstream gains
an equivalent counter.
