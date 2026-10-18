#!/bin/sh
# Build the GitHub Pages site into site/dist: inject the theme swatches from themes/*.toml
# and the repository and site URLs.
# Usage: scripts/build-site.sh [https://github.com/OWNER/REPO] [https://OWNER.github.io/REPO]
set -eu
cd "$(dirname "$0")/.."
REPO="${1:-${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:-OWNER/teryon}}"
SITE="${2:-https://zyvorai.github.io/teryon}"
rm -rf site/dist && mkdir -p site/dist
cp -R site/img site/favicon.svg site/dist/
cp site/img/apple-touch-icon.png site/dist/apple-touch-icon.png
python3 - "$REPO" "$SITE" <<'PY'
import glob, sys, tomllib
repo, site = sys.argv[1], sys.argv[2].rstrip("/")
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
html = open("site/index.template.html").read().replace("<!--SWATCHES-->", cards).replace("__REPO__", repo).replace("__SITE__", site)
open("site/dist/index.html", "w").write(html)
open("site/dist/robots.txt", "w").write(f"User-agent: *\nAllow: /\nSitemap: {site}/sitemap.xml\n")
open("site/dist/sitemap.xml", "w").write(
    '<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n'
    f"  <url><loc>{site}/</loc></url>\n</urlset>\n"
)
open("site/dist/404.html", "w").write(
    '<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">'
    '<title>Not found · TERYON</title><meta name="robots" content="noindex">'
    '<body style="margin:0;display:grid;place-items:center;min-height:100vh;background:#000;color:#f5f5f7;'
    'font:18px -apple-system,BlinkMacSystemFont,Helvetica,Arial,sans-serif;text-align:center">'
    f'<div><h1 style="font-size:56px;margin:0 0 8px">404</h1><p style="color:#a1a1a6">That page isn’t here.</p>'
    f'<p><a style="color:#2997ff" href="{site}/">Back to TERYON</a></p></div>'
)
PY
echo "site built in site/dist"
