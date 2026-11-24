#!/bin/zsh
# Publishes the `bucca` command that build.sh assembled: the platform packages first, then,
# once npm serves every one of their downloads, the `buccanet` launcher. npm lists a version a
# few minutes before it serves its download, and a launcher installed in that gap would come
# without its binary. npm asks for the account's second factor in the browser. The browser SDK
# (build-sdk.sh) is published on its own, with `npm publish --access public` in dist-sdk, once
# the repositories are public.
set -euo pipefail
cd "$(dirname "$0")"
VERSION=$(node -p 'require("./dist/buccanet/package.json").version')
platforms=(dist/buccanet-*(/))
for package in $platforms; do
  (cd $package && npm publish --access public)
done
for package in $platforms; do
  name=${package:t}
  url="https://registry.npmjs.org/$name/-/$name-$VERSION.tgz"
  until curl -sf -o /dev/null --head "$url"; do
    echo "waiting for npm to serve $name $VERSION…"
    sleep 15
  done
done
(cd dist/buccanet && npm publish --access public)
