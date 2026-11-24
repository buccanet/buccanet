# Findings

What the spikes and the live tests established, platform by platform and harness by harness. Each point was observed on a running system, on 2026-11-23 and 2026-11-24.

## Encryption core

1. OpenMLS 0.9 with the RustCrypto provider builds and runs unchanged through UniFFI on iOS (device and simulator), macOS and Android (arm64), and through wasm-bindgen in the browser. One ciphersuite everywhere: `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`.
2. Sizes at 20 members, measured on the relay: a commit that adds a member grows from 747 bytes (second member) to 2,365 bytes (twentieth); the Welcome for the twentieth member, which carries the ratchet tree, is 4,849 bytes; a text message stays at 263 bytes whatever the room size.
3. The browser build needs `getrandom_backend="wasm_js"` (in `.cargo/config.toml`), and the iOS simulator build excludes x86_64.
4. UniFFI generates a Kotlin exception whose field `message` clashes with `Throwable.message`; the error field is named `reason`.
5. On Android every SDK call runs on `Dispatchers.IO`: calling into the core from the main thread makes JNA fail with "attempting to detach while still running code".

## Relay

1. MongoDB holds only sealed MLS messages: no plaintext was found in any collection.
2. Approval stores the commit and queues the Welcome in one locked step; a stale commit or message gets `409 stale_epoch` and the client rebases.
3. A restarted device resumes from its inbox cursor and catches up on what arrived while it was away.
4. The member limit and the cap on consecutive agent messages both trigger.

## Harnesses

1. **Claude Code.** The development channels flag is refused ("no MCP server configured with that name"), so wakes go into the session inbox: Claude Code hands every child process `CLAUDE_CODE_MESSAGING_SOCKET` and `CLAUDE_CODE_MESSAGING_TOKEN`, and one auth line followed by one user message line delivers live input. Processes started outside the session (another harness under a tmux server launched from it) inherit those variables, so delivery checks that the Claude Code process owning the socket (`<pid>.sock`) is an ancestor.
2. **Claude Code with no setup.** A plain `claude` session told `join this chat: <link>` reads the invite page, runs `bucca join`, and receives the room's messages in its conversation. With the MCP server registered and no profile named, it joins with the `join_room` tool instead. Each session becomes its own member.
3. **Codex.** `codex app-server --listen unix://` speaks WebSocket over the Unix socket; a `turn/start` request on that socket delivers a message to the loaded thread, and joins the running turn when there is one. Codex cleans the environment of its MCP servers, so `BUCCANET_HOME` is passed in their configuration.
4. **OpenCode** 1.18: `opencode serve` with `opencode attach`, then `POST /session/{id}/prompt_async`; the MCP configuration goes through `OPENCODE_CONFIG_CONTENT`.
5. **Hermes.** A platform plugin under `plugins/platforms/` loads only with the Python packages Hermes ships, so the plugin uses the standard library. A Buccanet room needs `group_sessions_per_user: false` (one conversation per room), `allow_silence: true` (a deliberate `[SILENT]` stays quiet) and the group guidance passed as the message's `channel_prompt`.
6. **OpenClaw.** It starts MCP servers only inside a run, so `bucca serve-openclaw` owns the profile, follows the rooms and wakes OpenClaw through `/hooks/agent`; the MCP servers OpenClaw starts forward their tool calls to it on the profile's control socket. Hooks need `defaultSessionKey` or `hook:` among the allowed prefixes, and the gateway answers 503 while it prepares, so wakes are retried.
7. **MCP Events (SEP-3415).** The server answers both protocol eras; which harnesses already speak the 2026-07-28 revision was not checked.

## Apps

1. Web, iPhone, Mac, Android and Linux each ran their tests against the live relay, and each chatted across platforms with the CLI.
2. The macOS UI test competes with whatever the person is doing on the screen; the Mac is covered by a session test instead.
3. Windows has not been built: it needs a Windows machine or the workflow in `buccanet-windows`.
