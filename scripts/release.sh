#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'EOF'
Bump the Oryx Cargo package version, create a release commit, and tag it.

Usage:
  ./scripts/release.sh patch
  ./scripts/release.sh minor
  ./scripts/release.sh major

Behavior:
  - Requires a clean git worktree
  - Bumps the package version in Cargo.toml and Cargo.lock
  - Runs cargo check
  - Creates commit: release: vX.Y.Z
  - Creates annotated tag: vX.Y.Z

Notes:
  - Does not push commits or tags
  - Uses explicit semver bump kind; it does not infer intent
EOF
}

require_command() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Missing required command: $cmd" >&2
    exit 1
  fi
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -ne 1 ]]; then
  usage >&2
  exit 1
fi

BUMP_KIND="$1"
case "${BUMP_KIND}" in
  major|minor|patch) ;;
  *)
    echo "Invalid release bump '${BUMP_KIND}'. Expected: major, minor, or patch." >&2
    exit 1
    ;;
esac

require_command git
require_command cargo
require_command perl

cd "${ROOT_DIR}"

if ! git rev-parse --show-toplevel >/dev/null 2>&1; then
  echo "This script must be run inside a git repository." >&2
  exit 1
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "Git worktree is not clean. Commit or stash changes before releasing." >&2
  exit 1
fi

CURRENT_VERSION="$(
  sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1
)"

if [[ -z "${CURRENT_VERSION}" ]]; then
  echo "Could not determine current package version from Cargo.toml." >&2
  exit 1
fi

IFS='.' read -r MAJOR MINOR PATCH <<<"${CURRENT_VERSION}"
if [[ ! "${MAJOR}" =~ ^[0-9]+$ || ! "${MINOR}" =~ ^[0-9]+$ || ! "${PATCH}" =~ ^[0-9]+$ ]]; then
  echo "Current version '${CURRENT_VERSION}' is not simple semver major.minor.patch." >&2
  exit 1
fi

case "${BUMP_KIND}" in
  major)
    NEXT_VERSION="$((MAJOR + 1)).0.0"
    ;;
  minor)
    NEXT_VERSION="${MAJOR}.$((MINOR + 1)).0"
    ;;
  patch)
    NEXT_VERSION="${MAJOR}.${MINOR}.$((PATCH + 1))"
    ;;
esac

NEXT_TAG="v${NEXT_VERSION}"

if git rev-parse "${NEXT_TAG}" >/dev/null 2>&1; then
  echo "Tag ${NEXT_TAG} already exists." >&2
  exit 1
fi

OLD_VERSION="${CURRENT_VERSION}" NEW_VERSION="${NEXT_VERSION}" perl -0pi -e '
  s/^version = "\Q$ENV{OLD_VERSION}\E"$/version = "$ENV{NEW_VERSION}"/m
' Cargo.toml

if [[ -f Cargo.lock ]]; then
  OLD_VERSION="${CURRENT_VERSION}" NEW_VERSION="${NEXT_VERSION}" perl -0pi -e '
    s/(name = "oryx"\nversion = ")\Q$ENV{OLD_VERSION}\E(")/$1$ENV{NEW_VERSION}$2/
  ' Cargo.lock
fi

echo "Bumped version: ${CURRENT_VERSION} -> ${NEXT_VERSION}"

cargo check

git add Cargo.toml Cargo.lock
git commit -m "release: ${NEXT_TAG}"
git tag -a "${NEXT_TAG}" -m "${NEXT_TAG}"

cat <<EOF
Created release commit and tag:
  version: ${NEXT_VERSION}
  tag: ${NEXT_TAG}

Next step:
  git push && git push --tags
EOF
