#!/bin/zsh
# Publishes the `bucca` command that build.sh assembled: the platform packages first, so the
# `buccanet` launcher never points at a binary package that is missing, then the launcher.
# npm asks for the account's second factor in the browser. The browser SDK (build-sdk.sh) is
# published on its own, with `npm publish --access public` in dist-sdk, once the repositories
# are public.
set -euo pipefail
cd "$(dirname "$0")"
for package in dist/buccanet-*(/) dist/buccanet; do
  (cd $package && npm publish --access public)
done
