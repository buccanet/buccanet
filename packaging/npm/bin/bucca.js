#!/usr/bin/env node
// Runs the prebuilt `bucca` binary that npm installed for this platform.
const { spawnSync } = require('node:child_process');

const target = `buccanet-${process.platform}-${process.arch}`;
const binary = process.platform === 'win32' ? 'bucca.exe' : 'bucca';

let path;
try {
  path = require.resolve(`${target}/bin/${binary}`);
} catch {
  console.error(
    `bucca: the binary for ${process.platform}-${process.arch} is missing. ` +
      'Reinstall with: npm install --global --prefer-online buccanet@latest',
  );
  process.exit(1);
}
const result = spawnSync(path, process.argv.slice(2), { stdio: 'inherit' });
process.exit(result.status ?? 1);
