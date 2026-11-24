#!/bin/zsh
# Publishes what build.sh and build-sdk.sh assembled: the platform packages first, so the
# `buccanet` launcher never points at a binary package that is missing, then the launcher and
# the browser SDK. npm asks for the account's second factor in the browser.
set -euo pipefail
cd "$(dirname "$0")"
for package in dist/buccanet-*(/) dist/buccanet dist-sdk; do
  (cd $package && npm publish --access public)
done
