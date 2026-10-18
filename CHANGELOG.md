# Changelog

## 0.6.0 — One slim bar, Ghostty-style desktop

- The sidebar and status bar are gone: one 30 px top bar (window buttons merged into it on macOS) holds the workspace/tabs, pane chips, layout switch, New pane and the palette; messages are short toasts.
- Splits: weighted dividers for every layout, directional focus (⌥⌘ arrows), keyboard resize and equalize, new pane right/below, unfocused-pane dimming, exit notifications while the window is in the background.
- Terminal: word/line/block selection, shift-extend, copy-on-select, wheel scrollback from the daemon's retained text, xterm mouse reporting for vim/htop/tmux, wheel-as-arrows in full-screen programs, block/bar/underline cursor with blink.
- Config: typed `[desktop]` section in `config.toml` (font family and size, line height, padding, cursor, opacity, dimming, scrollback, pane titles, confirm quit) that reloads live; rebindable keys via `[desktop.keys]` with conflict detection; `teryon config`, `--check` and `--keys`.
- Palette shows live theme previews and gains next/previous/random theme, light/dark switch, fullscreen, always-on-top and reload-config actions. New `docs/DESKTOP.md`.
- Not supported (documented): ligatures, background blur, kitty protocols, OSC 8 links, shell-integration prompt jumps, native tabs and a global-hotkey quick terminal.

## 0.5.0 — Real macOS type, icon rail, compact bar

- Fonts: SF Pro now renders at true regular / semibold / bold weights (variable-font axis set through a small vendored `epaint` patch, see `vendor/epaint/PATCH.md`), SF Pro Rounded for section labels, SF Mono for terminals, and macOS sizes (13 px body, 11 px caption). Falls back to built-in fonts when the system files are absent.
- Sidebar has three states: full, a 46 px icon rail (pane chips plus new/find/copy/draft/theme icons) and hidden; ⌘B cycles them and narrow windows collapse automatically.
- One compact 36 px top bar with icon buttons and an icon layout switcher; per-pane title bars (double-click to rename, zoom, close, directory and exit chips); status bar shows latency, pane count, layout and font size.
- Command palette is a fuzzy-searchable action list with shortcut hints (themes, layouts, tabs, font size, find...); typed commands with arguments still work.
- Find bar (⌘F, ⌘G / ⇧⌘G, match counter), drag-to-select with ⌘C copy, ⌘-click opens http(s) links.
- Remembers window size, theme, font size, sidebar and layout in `~/.teryon/desktop.toml` (0600); ⌘1–9 jump between tabs, ⇧⌘←/→ reorder them, ⌘T / ⌘W open and close.

## 0.4.0 — Mac27 look

- New default `mac27-dark` (and `mac27-light`): macOS-style chrome in the desktop app (8px rounded filled buttons, source-list sidebar, segmented tab and layout controls, soft popup shadows, SF Pro Rounded section labels) and a system-color ANSI palette.
- Mac27 themes follow the macOS accent color (`AppleAccentColor`); primary buttons are darkened as needed to keep white text readable. Custom and other built-in themes are unchanged.
- Desktop sidebar can be hidden or shown (button or ⌘B) and resized by dragging its edge (140–420 px).
- GitHub Pages landing page in `site/` with a deploy workflow.
- `apple-dark` / `apple-light` remain available with their pill style.

## 0.3.0 — Apple look, Zyvor Z mark

- `apple-dark` (default) and `apple-light` themes from the netra apple.com contract; themes gain optional `primary` and 16-color `ansi` fields; every theme now paints ANSI 0-15 (derived when not defined), in both the TUI and the desktop.
- Desktop: SF Pro / SF Mono loaded from macOS at runtime when present, pill buttons with hairlines, a single blue primary action, raised popups, painted Zyvor Z mark.
- Logo and wordmark are now the Zyvor Z mark; the banner uses Zyvor orange.
- New shells get `CLICOLOR` and a tuned `LS_COLORS`/`LSCOLORS` when unset; the TUI footer rotates short tips; the header is graphite with a green status dot.
- `doctor HOST --fix --binary PATH [-y]` deploys a binary when no compatible daemon answers (asks first).
- `record HOST -w NAME --pane N -o FILE.cast` writes an asciicast v2 file (0600, never overwrites) from screen snapshots, not the raw byte stream.
- Desktop: workspace tabs (`tab NAME`, `closetab`, "New tab"), search hits tinted in the canvas, and a draggable divider for two-pane layouts.
- `themes --export wezterm` carries the 16 ANSI colors and, for Apple themes, an SF Mono font hint.

## 0.2.0 — renamed ZYLINK → TERYON

- New logo, wordmark, `.icns` app icon, and a gradient terminal banner (`teryon about`, `connect`; disable with `--no-splash`, `splash = false` or `TERYON_NO_SPLASH`).
- Tokyo Night (matching WezTerm's "Tokyo Night (Gogh)") is the default theme; added Tokyo Night Storm, Catppuccin Mocha, Dracula, Gruvbox Dark, Nord, Rosé Pine and Solarized Dark. `themes --export wezterm NAME` prints a WezTerm color block.
- TUI: single/vertical/horizontal/grid layouts (up to 4 panes visible), zoom, fuzzy pane switcher, close shortcut, OSC 52 yank from retained output, optional mouse focus, and a status line with layout, pane count, poll latency and clock.
- Desktop: layout selector for up to 4 panes, hover-to-preview themes, logo window icon and sidebar mark.
- New commands: `attach`, `dashboard`, `import-ssh`, `completions`, `about`; `exec --wait --notify`.
- Not included in this release: `doctor --fix`, session recording, desktop tabs, font-family picker, protocol ping.

- Crate, binary, environment variables (`TERYON_*`), config directory (`~/.config/teryon`) and runtime directory (`~/.teryon/run`) were renamed.
- `~/.config/zylink/config.toml` is still read when no TERYON config exists; it is never moved or modified.
- A pre-rename daemon and a renamed client use different sockets and will not attach to each other; restart remote services after upgrading.

## 0.1.0 — engineering preview

- Persistent per-user PTY service with authenticated local IPC.
- OpenSSH transport, reconnecting TUI and optional native GPU desktop client.
- Eight embedded themes and custom TOML theme files.
- Exclusive controllers, read-only viewers and ordered input acceptance.
- Pane controls, retained-output search/export and offline drafts.
- Durable tasks, directory/title layouts, rsync transfers and loopback forwards.
- Deployment/install tooling, Linux/macOS CI definitions and integration fixtures.
