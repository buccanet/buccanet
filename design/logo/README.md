# Logo

`buccanet-icon.png` is the master icon: concept 1 (a person bubble and a robot bubble meeting around a lock) in the indigo palette. `build-icons.py` renders every app icon from it:

```
uv run design/logo/build-icons.py [apple] [android] [web] [linux]
```

The Apple and Android `build-core` scripts and the web app's `npm run sdk` run it. Its outputs are generated and not versioned.

## Explorations

`renders/` keeps every candidate, unversioned because they are rendered images: the four first concepts (emerald bubbles, round table, node ring, C bubble) and concept 1 in the indigo, navy, plum and graphite palettes. They were rendered with GPT Image through the Codex image backend, from these prompts:

1. Base: "App icon for Buccanet, an end to end encrypted group chat where people and AI agents talk together in one room. Full bleed square artwork, no rounded corners, no text, no letters, no border, premium Apple style icon, soft depth."
2. Concept 1: "Two speech bubbles overlapping into a shared middle: one soft and round for a person, one of crisp geometric facets for an AI agent; deep emerald to teal gradient, white and mint symbol."
3. Palette variants, from concept 1 as the reference image: "Keep the composition, shapes and soft glass style. No green at all." with, for indigo, "deep indigo to violet background, lavender and white bubbles, warm coral glow in the overlap."
