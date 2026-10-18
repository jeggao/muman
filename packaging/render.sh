#!/usr/bin/env bash
# Fills the package manager entries beside this script with a release's
# version and its archives' checksums, as release.yml does:
#
#   packaging/render.sh <version> <SHA256SUMS> <out-dir>
#
# @VERSION@ becomes the version, and @SHA256:<target>@ the checksum of
# the archive built for that target; a placeholder left unfilled fails.
set -euo pipefail

version=$1
sums=$2
out=$3
here=$(cd "$(dirname "$0")" && pwd)

fill=(-e "s/@VERSION@/$version/g")
while read -r hash file; do
  target=${file#muman-v"$version"-}
  target=${target%.tar.gz}
  target=${target%.zip}
  fill+=(-e "s/@SHA256:$target@/$hash/g")
done < "$sums"

for entry in homebrew/muman.rb scoop/muman.json aur/PKGBUILD; do
  mkdir -p "$out/$(dirname "$entry")"
  sed "${fill[@]}" "$here/$entry" > "$out/$entry"
  if grep -n '@[A-Za-z0-9:_.-]*@' "$out/$entry"; then
    echo "$entry: a placeholder above has no value" >&2
    exit 1
  fi
done
