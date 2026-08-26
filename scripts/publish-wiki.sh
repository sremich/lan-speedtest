#!/usr/bin/env bash
# Publish docs/wiki/ to the repository's GitHub wiki.
#
# The wiki is a git repository of its own — <repo>.wiki.git — with a flat page
# namespace: `Home.md` is the landing page, `_Sidebar.md` is the navigation,
# and everything else is a page named after its file. That is already how
# docs/wiki/ is laid out, so publishing is a copy and a push.
#
#   scripts/publish-wiki.sh              # push
#   scripts/publish-wiki.sh --dry-run    # stage and show the diff, push nothing
#
# The target is derived from `origin`, so this keeps working across a repository
# rename without being edited. Override it with WIKI_REPO if you need to.
#
# BEFORE THE FIRST RUN: the wiki has to be initialised. Enabling it in the
# repository settings is not enough — until a first page is created through the
# web UI, <repo>.wiki.git does not exist and the clone below fails with
# "Repository not found", which reads like a permissions problem and is not.
set -euo pipefail

SRC=$(cd "$(dirname "$(readlink -f "$0")")/../docs/wiki" && pwd)
DRY_RUN=0
[[ ${1:-} == "--dry-run" ]] && DRY_RUN=1

command -v git >/dev/null || { echo "git is required" >&2; exit 1; }
[[ -f "$SRC/Home.md" ]] || { echo "no Home.md in $SRC" >&2; exit 1; }

if [[ -z ${WIKI_REPO:-} ]]; then
  origin=$(git -C "$SRC" remote get-url origin 2>/dev/null || true)
  [[ -n "$origin" ]] || { echo "no origin remote, and WIKI_REPO is unset" >&2; exit 1; }
  # Same URL, same credentials, ".wiki" before the ".git".
  WIKI_REPO="${origin%.git}.wiki.git"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

echo "==> cloning $WIKI_REPO"
if ! git clone --quiet "$WIKI_REPO" "$work/wiki" 2>"$work/err"; then
  cat "$work/err" >&2
  echo >&2
  echo "If that says 'Repository not found': the wiki exists in settings but" >&2
  echo "has no first page yet. Create one through the web UI, then re-run." >&2
  exit 1
fi

echo "==> replacing wiki contents from $SRC"
# Delete tracked content first so a page removed here is removed there too.
# This is a fresh clone, so there is nothing untracked to preserve.
(cd "$work/wiki" && git ls-files -z | xargs -0 -r rm -f)
cp "$SRC"/*.md "$work/wiki/"
# nullglob so an empty (or .png-less) images/ leaves the glob expanding to
# nothing rather than to a literal path that `cp` fails on — which under
# `set -e` would abort after the delete above had already run.
shopt -s nullglob
images=("$SRC"/images/*)
if (( ${#images[@]} )); then
  mkdir -p "$work/wiki/images"
  cp "${images[@]}" "$work/wiki/images/"
fi
shopt -u nullglob

cd "$work/wiki"
git add -A

if git diff --cached --quiet; then
  echo "    no changes"
  exit 0
fi

echo "==> changes"
git diff --cached --stat

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "==> dry run, nothing pushed"
  exit 0
fi

# The wiki is a fresh clone, so it inherits nothing from this repository's
# config — and on a machine whose git identity is only ever set per-repository,
# committing here fails outright. Carry the source repository's identity across,
# falling back to GitHub's noreply form rather than whatever the host guessed
# from the hostname.
wiki_name="$(git -C "$SRC" config user.name || true)"
wiki_email="$(git -C "$SRC" config user.email || true)"
git config user.name "${wiki_name:-$(git config --global user.name || echo "wiki publisher")}"
git config user.email "${wiki_email:-$(git config --global user.email || echo "noreply@users.noreply.github.com")}"

git commit --quiet -m "Publish wiki from docs/wiki"
git push --quiet origin HEAD
echo "==> pushed"

cat <<'NOTE'

Check one image on the published wiki before considering this done. GitHub
serves wiki assets from the wiki repository, and a relative `images/foo.png`
link does not render on every wiki. If the images are broken, rewrite them to
the raw form, which always resolves:

  https://raw.githubusercontent.com/wiki/<owner>/<repo>/images/foo.png
NOTE
