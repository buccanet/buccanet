#!/bin/zsh
# Assembles the npm packages in packaging/npm/dist: one package per platform holding the `bucca`
# binary, and the `buccanet` launcher depending on the platforms built here. Every package takes
# the version of cli/Cargo.toml, so `bucca --version` and npm always agree.
#
#   packaging/npm/build.sh <rust target>...
#   packaging/npm/build.sh aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu
#
# Linux binaries are cross built with cargo-zigbuild against glibc 2.28, so they run on any
# distribution from 2018 on. Publish afterwards with `npm publish` in each dist folder,
# platforms first.
set -euo pipefail
cd "$(dirname "$0")/../.."
METADATA=$(cargo metadata --no-deps --format-version 1)
VERSION=$(node -p 'JSON.parse(process.argv[1]).packages.find(p => p.name === "buccanet-cli").version' $METADATA)
# The build folder as Cargo resolves it: CARGO_TARGET_DIR, a .cargo/config.toml above, or target/.
TARGET_DIR=$(node -p 'JSON.parse(process.argv[1]).target_directory' $METADATA)
DIST=packaging/npm/dist
rm -rf $DIST && mkdir -p $DIST

# Rust target to npm's os and cpu names.
typeset -A OS=(aarch64-apple-darwin darwin x86_64-apple-darwin darwin x86_64-unknown-linux-gnu linux aarch64-unknown-linux-gnu linux x86_64-pc-windows-msvc win32)
typeset -A CPU=(aarch64-apple-darwin arm64 x86_64-apple-darwin x64 x86_64-unknown-linux-gnu x64 aarch64-unknown-linux-gnu arm64 x86_64-pc-windows-msvc x64)

platforms=()
for target in "$@"; do
  os=${OS[$target]:?unknown target $target}
  cpu=${CPU[$target]}
  name=buccanet-$os-$cpu
  binary=bucca
  [[ $os == win32 ]] && binary=bucca.exe
  case $os in
    linux) cargo zigbuild -p buccanet-cli --release --target $target.2.28 ;;
    *) cargo build -p buccanet-cli --release --target $target ;;
  esac
  mkdir -p $DIST/$name/bin
  cp $TARGET_DIR/$target/release/$binary $DIST/$name/bin/
  cp LICENSE $DIST/$name/
  cat > $DIST/$name/package.json <<JSON
{
  "name": "$name",
  "version": "$VERSION",
  "description": "The bucca binary for $os $cpu. Install buccanet instead.",
  "license": "Apache-2.0",
  "os": ["$os"],
  "cpu": ["$cpu"],
  "files": ["bin"]
}
JSON
  platforms+=("\"$name\": \"$VERSION\"")
done

mkdir -p $DIST/buccanet/bin
cp packaging/npm/bin/bucca.js $DIST/buccanet/bin/
cp packaging/npm/README.md LICENSE $DIST/buccanet/
node -e '
const fs = require("fs");
const [source, out, version, deps] = process.argv.slice(1);
const manifest = JSON.parse(fs.readFileSync(source, "utf8"));
manifest.version = version;
manifest.optionalDependencies = JSON.parse(`{${deps}}`);
fs.writeFileSync(out, JSON.stringify(manifest, null, 2) + "\n");
' packaging/npm/package.json $DIST/buccanet/package.json $VERSION "${(j:, :)platforms}"
echo "npm packages ready in $DIST: ${(j: :)${(f)$(ls $DIST)}}"
