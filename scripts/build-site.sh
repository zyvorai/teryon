#!/bin/sh
# Build the GitHub Pages site into site/dist: inject the theme swatches from themes/*.toml
# and the repository URL. Usage: scripts/build-site.sh [https://github.com/OWNER/REPO]
set -eu
cd "$(dirname "$0")/.."
REPO="${1:-${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:-OWNER/teryon}}"
rm -rf site/dist && mkdir -p site/dist
cp -R site/img site/favicon.svg site/dist/
python3 - "$REPO" <<'PY'
import glob, sys, tomllib
repo = sys.argv[1]
order = ["Mac27 Dark", "Mac27 Light", "Apple Dark", "Apple Light"]
themes = []
for f in glob.glob("themes/*.toml"):
    with open(f, "rb") as fh:
        themes.append(tomllib.load(fh))
themes.sort(key=lambda t: (order.index(t["name"]) if t["name"] in order else 99, t["name"]))
cards = "".join(
    f'<div class="sw" style="background:{t["background"]};color:{t["foreground"]};border-color:{t["panel"]}">'
    f'<b>{t["name"]}</b><span>'
    + "".join(f'<i style="background:{t[k]}"></i>' for k in ("accent", "success", "warning"))
    + "</span></div>\n"
    for t in themes
)
html = open("site/index.template.html").read().replace("<!--SWATCHES-->", cards).replace("__REPO__", repo)
open("site/dist/index.html", "w").write(html)
PY
echo "site built in site/dist"
