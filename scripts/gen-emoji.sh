#!/bin/sh
# Regenerates the emoji shortcode table (crates/hearth-model/src/emoji.txt) from
# GitHub's gemoji: the `:joy:` names Slack and Discord also use. Needs curl and jq.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
out="$root/crates/hearth-model/src/emoji.txt"
curl -fsSL https://raw.githubusercontent.com/github/gemoji/master/db/emoji.json |
  jq -r '.[] | .emoji as $e | .aliases[] | "\(.) \($e)"' > "$out.tmp"
mv "$out.tmp" "$out"
echo "$(wc -l < "$out" | tr -d ' ') shortcodes"
