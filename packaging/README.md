# Packaging

| Channel | What | How |
| --- | --- | --- |
| npm | `buccanet` (command `bucca`) with one prebuilt binary package per platform | `packaging/npm`; each platform package holds `bin/bucca` built with `cargo build -p buccanet-cli --release --target <triple>` |
| Homebrew | `buccanet` formula built from source | `packaging/homebrew/buccanet.rb` |
| cargo | `cargo install --path cli` | the crate `buccanet-cli` |
| Docker | relay and web app on one origin | `docker build -f relay/Dockerfile -t buccanet-relay:dev .` |
