# Themes

Eighteen themes are embedded in the executable, so changing machines does not require installing assets:

| CLI name | Style |
|---|---|
| `mac27-dark` (default) | macOS-style: `#1e1e1e` canvas, `#2a2a2c` sidebar/toolbar, 8px rounded buttons, segmented tabs/layout, follows the macOS accent color (System Settings > Appearance) |
| `mac27-light` | White canvas, `#f2f2f7` chrome, same shapes; follows the macOS accent color |
| `apple-dark` | netra/apple.com dark: black page, `#1d1d1f` panels, `#f5f5f7` text, blue `#0071e3` primary pill, graphite chrome |
| `apple-light` | White page, `#f5f5f7` panels, text-safe blue/green/amber accents |
| `tokyo-night` | Matches WezTerm's "Tokyo Night (Gogh)": deep navy, blue accent, green status |
| `tokyo-night-storm` | Lighter slate variant of Tokyo Night |
| `tokyo-night-day` | Light Tokyo Night with darkened text so it passes the contrast test |
| `carbon` | Neutral near-black (#161616) with a soft blue accent |
| `catppuccin-mocha` | Warm mauve on dark chocolate |
| `dracula` | Purple accent, neon green status |
| `gruvbox-dark` | Retro warm browns and yellow |
| `nord` | Arctic blue-grey with frost cyan |
| `rose-pine` | Muted violet with soft gold |
| `solarized-dark` | Classic teal-dark with blue accent |
| `obsidian` | Graphite, violet selection, mint status |
| `pearl` | Warm light background and dark readable text |
| `aurora` | Blue-grey surfaces and icy cyan |
| `ember` | Warm charcoal and peach |
| `ion` | Void navy, ion-blue accent, amber for "needs you" |
| `ocean` | Deep navy and bright blue |
| `sakura` | Plum and pink |
| `forest` | Dark green and sage |
| `high-contrast` | Black, white and bright status colors |

```bash
teryon themes --preview
teryon connect mini --theme pearl
teryon desktop mini --theme aurora
```

## File-type colors

New shells start with `CLICOLOR=1`, a tuned `LS_COLORS`/`LSCOLORS` and an `EZA_COLORS` for `eza` (directories bold blue, executables bold green, symlinks cyan, archives red, images magenta, video bold magenta, audio cyan, source code yellow, config and data bright cyan, documents white, lock files and build leftovers dim; permission, size, user and date columns in `eza -l` get their own colors). They are ANSI indexes, so every theme repaints them. Variables you already set are left alone, and a rc file can override them.

Cycle themes in the TUI with Ctrl+Space followed by `t`. In either command palette, run `theme ocean`. In the desktop sidebar, use the theme picker. These changes affect the current client; set the preferred startup theme in `~/.config/teryon/config.toml`:

```toml
theme = "pearl"
```

Export a theme as a WezTerm `config.colors` block (paste it into `wezterm.lua` after `config` is created):

```bash
teryon themes --export wezterm tokyo-night
```

The block carries the theme's 16 ANSI colors. For the Apple themes it also adds a commented `config.font` hint for SF Mono (install it from developer.apple.com/fonts; Menlo is built in). Your `wezterm.lua` is never edited.

Use `teryon init` to create a complete starter configuration without overwriting an existing file.

## Custom themes

Copy one of the `themes/*.toml` files to a local path and edit all eight color fields. Two optional fields exist: `primary` (the solid fill of the desktop's primary button, defaults to `accent`) and `ansi` (exactly sixteen colors for ANSI 0-15, derived from the other colors when absent). Colors must use `#RRGGBB`. Unknown fields and malformed colors are rejected.

```bash
teryon connect mini --theme /Users/susant/.config/teryon/my-theme.toml
```

Theme files are loaded locally. They contain no scripts. They style app chrome and default terminal colors; explicit colors emitted by remote applications retain their ANSI/true-color meaning. Dark-oriented shell themes may need their own light variant when using Pearl.

The desktop font size slider ranges from 11 to 24 points. Terminal glyph coverage depends on the frontend's available fonts; this preview does not ship a complete CJK or color-emoji font stack. The TUI uses your terminal emulator's fonts.

## Your own themes and imports

Theme files in `~/.config/teryon/themes/NAME.toml` are picked up by name (`--theme NAME`, `theme = "NAME"`,
the palette). `teryon themes --import FILE` converts a scheme from another terminal into one: iTerm2
`.itermcolors`, Alacritty `.toml`, and Ghostty or Kitty config files (the format is detected). Missing palette
colors are filled with xterm defaults, secondary text is lightened until it reaches a 4.5:1 contrast, and the
command warns when the scheme's own foreground/background contrast is lower than that.

With `theme_light` / `theme_dark` in the `[desktop]` section the desktop follows the macOS appearance.
