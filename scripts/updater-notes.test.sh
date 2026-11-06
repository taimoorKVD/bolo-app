#!/bin/sh
# Proves a shallow checkout cannot leak the commit message into updater notes.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT
bare="$tmp/origin.git"
work="$tmp/work"
shallow="$tmp/shallow"
git init --bare "$bare" >/dev/null
git init "$work" >/dev/null
git -C "$work" config user.email "notes@example.com"
git -C "$work" config user.name "Notes Test"
mkdir -p "$work/docs/release-notes"
printf '%s\n' "# Bolo 9.9.9" "" "Annotated fallback from the notes file." > "$work/docs/release-notes/9.9.9.md"
echo "seed" > "$work/README"
git -C "$work" add .
git -C "$work" commit -m "$(printf '%s\n' 'Bump Bolo to 9.9.9 and write the release notes.' '' 'This commit message must not become the updater note.')" >/dev/null
git -C "$work" remote add origin "$bare"
git -C "$work" push -u origin HEAD:main >/dev/null
git -C "$work" tag -a v9.9.9 -m "$(printf '%s\n' 'A normal sentence stays what you said.' '' '- Quiet speech is lifted.')"
git -C "$work" push origin v9.9.9 >/dev/null

git clone --depth 1 --branch main "file://$bare" "$shallow" >/dev/null
# Same shape as actions/checkout: the commit is present, the annotated tag is not.
git -C "$shallow" tag -d v9.9.9 >/dev/null 2>&1 || true
git -C "$shallow" tag v9.9.9
kind=$(git -C "$shallow" cat-file -t refs/tags/v9.9.9)
if [ "$kind" != "commit" ]; then
  echo "expected a lightweight tag before fetch, got $kind" >&2
  exit 1
fi
leaked=$(git -C "$shallow" for-each-ref --format='%(contents)' refs/tags/v9.9.9)
case "$leaked" in
  *"Bump Bolo to 9.9.9"*) ;;
  *) echo "fixture did not reproduce the commit-message leak" >&2; exit 1 ;;
esac

notes=$(sh "$root/scripts/updater-notes.sh" "$shallow" v9.9.9)
case "$notes" in
  *"A normal sentence stays what you said."*) ;;
  *) echo "annotated tag was not read:" >&2; printf '%s\n' "$notes" >&2; exit 1 ;;
esac
case "$notes" in
  *"Bump Bolo to 9.9.9"*) echo "commit message leaked into notes" >&2; exit 1 ;;
esac

# No tag object and no notes file: still not the commit message.
rm -rf "$shallow"
git clone --depth 1 --branch main "file://$bare" "$shallow" >/dev/null
git -C "$shallow" tag v0.0.1
missing=$(sh "$root/scripts/updater-notes.sh" "$shallow" v0.0.1)
case "$missing" in
  "Bolo 0.0.1") ;;
  *) echo "missing tag should fall back to Bolo <version>, got:" >&2; printf '%s\n' "$missing" >&2; exit 1 ;;
esac
echo "updater-notes: pass"
