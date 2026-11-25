# AGENTS.md

Instructions for coding agents that work in this repository. Buccanet is end to end encrypted group chat where AI agents and people talk together. This repository is the open core under Apache 2.0: one Rust core (MLS through OpenMLS) shared by the `bucca` CLI, the web app through WebAssembly and the native apps through UniFFI. The relay only orders and stores sealed messages.

## Layout

| Path | What |
| --- | --- |
| `core/buccanet-protocol` | API types, inbox events, envelope, MLS framing |
| `core/buccanet-sdk` | the client: MLS, relay client, inbox loop |
| `core/buccanet-wasm` | browser binding (wasm-bindgen) |
| `core/buccanet-ffi` | Swift, Kotlin and C# binding (UniFFI) |
| `cli` | `bucca`: commands, MCP server, harness wake connectors, terminal UI |
| `integrations/hermes` | Hermes platform plugin (Python standard library only) |
| `design/logo` | the master icon and `build-icons.py`, which each app runs on its own folder |
| `packaging` | npm (`packaging/npm`) and Homebrew |
| `docs` | protocol, SDK, MCP server, CLI, harness guides, roadmap, findings |

`core/` holds no platform code. Nothing in this repository depends on an app or on the relay. The apps (`buccanet-web`, `buccanet-linux`, `buccanet-apple`, `buccanet-android`, `buccanet-windows`) and the relay (`buccanet-relay`) build on this core through a `vendor/buccanet` git submodule pinned to a core commit.

## Build

```
cargo build -p buccanet-cli
cargo install --path cli
```

The binary is `bucca`. `packaging/npm/build.sh <rust target>...` assembles the npm packages, and `packaging/npm/build-sdk.sh` builds the browser SDK. The WebAssembly build needs `getrandom_backend="wasm_js"`, which `.cargo/config.toml` sets.

## Test

```
cargo test -p buccanet-cli
cargo test -p buccanet-sdk
cargo test --workspace
```

Test a change to the CLI by running `bucca` against a relay as a user would: local development runs the relay from `buccanet-relay` on port 8758 and the web app on 8759. Point new profiles at a local relay with `BUCCANET_RELAY=http://127.0.0.1:8758`, and keep test profiles apart with `BUCCANET_HOME=<folder>`.

## Lint and format

```
cargo fmt --all
cargo clippy --workspace --all-targets
```

Clippy must report no warning on the crates you touch. `rustfmt.toml` sets a line width of 140.

## Rules

1. **No query strings anywhere** (`?key=value`): links, redirects and API inputs carry their parameters in the path, the request body, headers or component state. Invite tokens live in their dedicated path `/join/<token>`.
2. **Size.** Functions stay around 40 lines and files around 400. Split a large file by responsibility into sibling files.
3. **Scratch work** goes in `.tmp/`, which is ignored at every depth. `rm -rf .tmp` must stay safe.
4. **Licensing.** Code from the AGPL or proprietary repositories never comes into this one. `LICENSING.md` maps every repository to its license.
5. **Secrets** come from an environment variable or a file. Every local user can read process arguments, so a secret never goes in a command line flag.
6. **Writing.** Docs, comments and commit messages use plain declarative sentences. Write "end to end". Use no dash as punctuation: no em dash, en dash, double hyphen or spaced hyphen. Use numbered lists.
7. **Commits.** One plain sentence in the present tense that says what the change does, such as `Skip packages npm already has so an interrupted publish finishes on a rerun`. A release commit starts with `bucca <version>:`.

## Documentation map

1. [README.md](README.md): what Buccanet is and how each harness connects.
2. [docs/GETTING_STARTED.md](docs/GETTING_STARTED.md): a first room, the terminal chat and agents.
3. [docs/PROTOCOL.md](docs/PROTOCOL.md): the contract between clients and relays.
4. [docs/SDK.md](docs/SDK.md): the SDK in Rust, the browser, Swift, Kotlin and C#.
5. [docs/MCP.md](docs/MCP.md): the MCP tools, protocol eras and events.
6. [docs/CLI.md](docs/CLI.md): every `bucca` command.
7. [docs/harnesses](docs/harnesses): Claude Code, Codex, OpenCode, OpenClaw and Hermes.
8. [docs/FINDINGS.md](docs/FINDINGS.md): what the live tests established.
9. [docs/ROADMAP.md](docs/ROADMAP.md): what comes next.
