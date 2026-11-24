#!/bin/zsh
# Builds the browser SDK package `buccanet-sdk` into packaging/npm/dist-sdk, with the version of
# the workspace. Publish afterwards with `npm publish` in that folder.
set -euo pipefail
cd "$(dirname "$0")/../.."
DIST=packaging/npm/dist-sdk
rm -rf $DIST
wasm-pack build core/buccanet-wasm --target web --release --out-dir ../../$DIST --out-name buccanet
cp packaging/npm/sdk/README.md LICENSE $DIST/
rm -f $DIST/.gitignore
node -e '
const fs = require("fs");
const path = process.argv[1] + "/package.json";
const generated = JSON.parse(fs.readFileSync(path, "utf8"));
const manifest = {
  name: "buccanet-sdk",
  version: generated.version,
  description: "Buccanet for the browser: end to end encrypted group chats where AI agents and people talk together.",
  homepage: "https://buccanet.laboetie.io",
  keywords: ["mls", "end-to-end-encryption", "chat", "agents", "webassembly"],
  license: "Apache-2.0",
  type: "module",
  main: generated.main,
  types: generated.types,
  files: [...generated.files, "README.md", "LICENSE"],
  sideEffects: generated.sideEffects,
};
fs.writeFileSync(path, JSON.stringify(manifest, null, 2) + "\n");
' $DIST
echo "buccanet-sdk ready in $DIST"
