# Blocks Not Included

Keep simulation rules in `crates/sim` and terrain generation in `crates/worldgen`. Both crates must stay Bevy-free. The Bevy application lives in `src/` and communicates with the background simulation through `src/sim_bridge.rs`.

Use [CONTEXT.md](CONTEXT.md) for domain terminology. Consult [architecture decisions](docs/adr/) when changing simulation boundaries, Cell representation or World saves. For a numbered implementation step, read its [plan](docs/plans/) and the [shared conventions](docs/plans/CONVENTIONS.md) for test-first development and per-Tick invariants. For Bevy changes, consult the [version-specific notes](docs/plans/BEVY-0.20-NOTES.md).

## Development commands

- Run the game with `cargo run`. The default development feature enables asset watching.
- Run focused library tests with `cargo test -p sim` or `cargo test -p worldgen`. Use `cargo test --workspace` to include the game and both libraries.
- Build the release game with `cargo build --release --no-default-features`.

## Documentation lookups

<!-- context7 -->
Use Context7 MCP for current documentation when a task depends on a library, framework, SDK, API, CLI tool or cloud service, including setup, configuration, migrations, porting and API-specific debugging. Prefer Context7 over web search for library docs. Skip it for general programming concepts and for refactoring, debugging or review independent of platform or library APIs.

1. Start with `resolve-library-id` using the library name and lookup question, unless the user supplies an exact library ID in `/org/project` format.
2. Select by name match, relevance, snippet count, source reputation and benchmark score. Use a version-specific ID when a version is specified. Retry with an alternate name or query if the matches are poor.
3. Call `query-docs` with the selected ID and a specific question. Query distinct concepts separately unless their interaction is the question.
4. Base the answer on the retrieved documentation.
<!-- context7 -->
