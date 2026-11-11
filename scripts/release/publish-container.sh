#!/usr/bin/env bash
set -euo pipefail

: "${IMAGE:?Container image required}"
: "${GITHUB_SHA:?Commit SHA required}"
: "${GITHUB_REF:?Git ref required}"
case "$GITHUB_REF" in
  refs/heads/main|refs/tags/v*) ;;
  *) echo "Only main and version tags can publish" >&2; exit 1;;
esac

# Preserve immutable tags even when main has advanced during the build.
docker push "$IMAGE:sha-$GITHUB_SHA"
if [[ "$GITHUB_REF" == refs/tags/v* ]]; then
  docker push "$IMAGE:${GITHUB_REF#refs/tags/v}"
fi
case "$GITHUB_REF" in
  refs/tags/v*-*) exit 0;;
  refs/heads/main)
    main_sha=$(gh api "repos/${GITHUB_REPOSITORY:?Repository required}/git/ref/heads/main" --jq .object.sha)
    if [ "$main_sha" != "$GITHUB_SHA" ]; then
      echo "Main advanced; keeping this image addressable by its commit only."
      exit 0
    fi;;
  refs/tags/v*) ;;
esac
docker tag "$IMAGE:sha-$GITHUB_SHA" "$IMAGE:latest"
docker push "$IMAGE:latest"
