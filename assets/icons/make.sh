#!/usr/bin/env bash
#
# Rasterises the menu's icons from two icon libraries into the PNGs next to
# this script, which src/devicons.rs and src/glyphs.rs embed in the binary.
#
#   Devicon  (MIT)  https://devicon.dev      the logos of languages, frameworks
#                                            and servers: tech/<name>.png
#   Lucide   (ISC)  https://lucide.dev       line glyphs for menu actions:
#                                            glyph/<name>.png, black on clear,
#                                            tinted at runtime for the theme
#
# Needs npm (to fetch the pinned packages) and rsvg-convert (brew install
# librsvg). The output is committed, so building Porta needs neither.
#
#   assets/icons/make.sh
set -euo pipefail
cd "$(dirname "$0")"

DEVICON=2.17.0
LUCIDE=0.545.0
EDGE=32   # menus draw icons at 16 pt: 32 px is sharp on Retina and at 200%
PAD=2     # breathing room, so logos sit at the size of the OS's own icons

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
(cd "$work" && npm pack --silent "devicon@$DEVICON" "lucide-static@$LUCIDE" >/dev/null)
mkdir -p "$work/devicon" "$work/lucide"
tar xzf "$work/devicon-$DEVICON.tgz" -C "$work/devicon"
tar xzf "$work/lucide-static-$LUCIDE.tgz" -C "$work/lucide"

raster() { # svg png
  local inner=$((EDGE - 2 * PAD))
  rsvg-convert -w "$inner" -h "$inner" --keep-aspect-ratio \
    --page-width "$EDGE" --page-height "$EDGE" --left "$PAD" --top "$PAD" \
    -o "$2" "$1"
}

# name in Porta = Devicon folder[:variant]. "original" is the full-colour logo;
# "plain" where original is a wordmark or missing.
TECH=(
  # runtimes and languages
  nodejs denojs bun python ruby go rust java kotlin scala php
  dotnetcore elixir erlang swift dart haskell perl lua clojure zig crystal
  julia r:plain ocaml
  # web frameworks and dev servers
  nextjs vitejs react angular vuejs nuxtjs svelte astro remix gatsby nestjs
  express fastify fastapi django:plain flask rails:plain laravel symfony
  spring phoenix storybook webpack vitest electron expo ember solidjs qwik
  hugo jekyll jupyter streamlit flutter rollup
  apollographql prisma
  # databases, queues and servers
  postgresql mysql mariadb mongodb redis nginx apache tomcat
  elasticsearch rabbitmq apachekafka grafana prometheus couchdb cassandra
  neo4j clickhouse memcached vault consul jenkins
  # tooling and platforms
  docker podman kubernetes gradle maven cloudflare vercel netlify firebase
  supabase ngrok
)

rm -rf tech glyph
mkdir -p tech glyph
for entry in "${TECH[@]}"; do
  name="${entry%%:*}"
  variant=original
  [[ "$entry" == *:* ]] && variant="${entry#*:}"
  src="$work/devicon/package/icons/$name/$name-$variant.svg"
  [ -f "$src" ] || src="$work/devicon/package/icons/$name/$name-plain.svg"
  raster "$src" "tech/$name.png"
done

# Porta name = Lucide name.
GLYPHS=(
  globe copy circle-stop octagon-x terminal clock wifi house folder-open
  refresh-cw file-cog power
)
for name in "${GLYPHS[@]}"; do
  raster "$work/lucide/package/icons/$name.svg" "glyph/$name.png"
done

echo "$(ls tech | wc -l | tr -d ' ') tech logos, $(ls glyph | wc -l | tr -d ' ') glyphs"
