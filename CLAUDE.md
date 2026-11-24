# Project rules

## What this is

Buccanet: end to end encrypted group chats where AI agents and people talk together, on every platform. One Rust core (MLS through OpenMLS), in the `buccanet` repository, is shared by the CLI, the web app (WebAssembly) and the native apps (UniFFI). The relay only orders and stores sealed messages. Local dev: relay on 8758, web app on 8759. The protocol, SDK and roadmap live in `buccanet/docs`.

## Repositories

`buccanet` is the open core. `buccanet-relay`, `buccanet-web`, `buccanet-linux`, `buccanet-apple`, `buccanet-android` and `buccanet-windows` each build on it through the `vendor/buccanet` git submodule, pinned to a core commit. Locally the seven repositories sit side by side in one folder.

## No query strings, anywhere

Query strings (`?key=value`) are forbidden across every repository: apps, relay, redirects and internal links. State travels in the path, in the request body, in headers or in component state. External URLs we call are exempt.

- Links and redirects carry their parameters as path segments.
- API inputs are path parameters or JSON bodies.
- Tokens never appear in a URL, except opaque tokens in their dedicated path: invite tokens in `/join/<token>` are allowed because they are reusable by design, expire after 7 days, can be revoked, and every join they start still waits for an admin.

## Layout

- `core/`: the Rust crates every client uses; no platform code there.
- `cli/`: commands, MCP server, harness wake connectors, terminal UI.
- `integrations/`: harness plugins (Hermes).
- `design/logo/`: the master icon and `build-icons.py`, which each app runs on its own folder.
- `packaging/`: npm and Homebrew.
- `docs/`: protocol, SDK, roadmap.
- Nothing here depends on an app or on the relay.

Functions stay around 40 lines, files around 400. Scratch work goes in `.tmp/`.

## Licensing

Apache 2.0. Code from the AGPL or proprietary repositories never comes into this one. `LICENSING.md` maps every repository to its license.

## Writing Rules

All copy and writing must follow the writing rules in `/Users/ilan/Projects/utils/skillslib/plugins/ilan-skills/skills/_shared/references/writing-quality.md`.
