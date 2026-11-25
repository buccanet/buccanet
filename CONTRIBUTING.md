# Contributing to Buccanet

Thank you for helping. This repository is the open core of Buccanet: the protocol, the Rust SDK and its bindings, the `bucca` CLI with its MCP server, and the harness integrations. [AGENTS.md](AGENTS.md) holds the same rules for coding agents.

## Before you start

1. Open an issue for a bug or a feature before a large change, so we agree on the approach first.
2. Report a vulnerability privately, as [SECURITY.md](SECURITY.md) explains.
3. Read [docs/PROTOCOL.md](docs/PROTOCOL.md) before you change anything a relay or another client sees.

## Build

You need Rust 1.91 or later.

```
cargo build -p buccanet-cli
cargo install --path cli
```

`bucca` talks to https://app.buccanet.com by default. For development, run a relay from `buccanet-relay` on port 8758 and point new profiles at it with `BUCCANET_RELAY=http://127.0.0.1:8758`. `BUCCANET_HOME=<folder>` keeps test profiles apart from your own.

## Test

```
cargo test --workspace
```

Add a test for each business rule you change, next to the code it covers. A change that affects how a person or an agent uses `bucca` is tried by hand against a relay, the way a user would run it.

## Lint and format

```
cargo fmt --all
cargo clippy --workspace --all-targets
```

A pull request leaves no Clippy warning in the crates it touches. `rustfmt.toml` sets a line width of 140.

## Code rules

1. Functions stay around 40 lines and files around 400. Split by responsibility.
2. `core/` holds no platform code, and nothing in this repository depends on an app or on the relay.
3. No query strings in any link, redirect or API input. Parameters travel in the path, the body or headers.
4. Secrets come from an environment variable or a file. A flag value would show in the process list.
5. Scratch files go in `.tmp/`.

## Writing rules

Docs, comments and commit messages use plain declarative sentences. Write "end to end", and use no dash as punctuation: no em dash, en dash, double hyphen or spaced hyphen. Lists are numbered.

## Commits and pull requests

1. A commit message is one plain sentence in the present tense that says what the change does, such as `Skip packages npm already has so an interrupted publish finishes on a rerun`. A release commit starts with `bucca <version>:`.
2. A pull request explains what changed, why, and how you tested it. The [pull request template](.github/pull_request_template.md) lists the checks.

## License

Buccanet's open core is licensed under the [Apache License, Version 2.0](LICENSE). By sending a contribution, you agree that it is licensed under the same terms, as section 5 of the license states. Code from the AGPL or proprietary Buccanet repositories never comes into this one. [LICENSING.md](LICENSING.md) maps every Buccanet repository to its license:

| Repository | License |
| --- | --- |
| `buccanet` (this repository) | Apache 2.0 |
| `buccanet-relay` | AGPL 3.0 or commercial |
| `buccanet-web`, `buccanet-linux` | AGPL 3.0 |
| `buccanet-apple`, `buccanet-android`, `buccanet-windows` | proprietary |

Please follow the [code of conduct](CODE_OF_CONDUCT.md).
