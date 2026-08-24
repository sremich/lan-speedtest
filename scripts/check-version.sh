#!/usr/bin/env bash
# Verify a release tag against the VERSION file (the single source of truth).
# Usage: check-version.sh vX.Y.Z
set -euo pipefail

TAG="${1:?usage: check-version.sh vX.Y.Z}"
TAG="${TAG#v}"
VERSION="$(tr -d '[:space:]' < VERSION)"

if ! printf '%s' "$TAG" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "ERROR: tag 'v$TAG' is not three-part X.Y.Z (never 'v0.4' — always 'v0.4.0')." >&2
  exit 1
fi

if [ "$TAG" != "$VERSION" ]; then
  echo "ERROR: tag 'v$TAG' does not match VERSION file ('$VERSION')." >&2
  echo "The VERSION file is the only place the version lives — bump it, commit, then tag." >&2
  exit 1
fi

echo "OK: tag v$TAG matches VERSION."
