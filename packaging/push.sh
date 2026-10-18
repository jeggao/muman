#!/usr/bin/env bash
# Commits one rendered entry to a package manager's repository on GitHub
# and pushes it, as release.yml does for the Homebrew tap and the Scoop
# bucket; TOKEN must be able to write to that repository.
#
#   packaging/push.sh <owner/repo> <file> <path-in-repo> <message>
set -euo pipefail

repo=$1
file=$2
path=$3
message=$4
clone=$(mktemp -d)

git clone --depth 1 "https://x-access-token:${TOKEN}@github.com/${repo}.git" "$clone"
git -C "$clone" config user.name "github-actions[bot]"
git -C "$clone" config user.email "41898282+github-actions[bot]@users.noreply.github.com"
mkdir -p "$clone/$(dirname "$path")"
cp "$file" "$clone/$path"
git -C "$clone" add "$path"
if git -C "$clone" diff --cached --quiet; then
  echo "$repo already has this $path"
  exit 0
fi
git -C "$clone" commit -m "$message"
# The tap and the bucket may be one repository, pushed to by two jobs at
# once; the one pushing second takes the other's commit and tries again.
for attempt in 1 2 3; do
  git -C "$clone" push origin HEAD && exit 0
  [ "$attempt" = 3 ] && exit 1
  git -C "$clone" pull --rebase origin "$(git -C "$clone" rev-parse --abbrev-ref HEAD)"
done
