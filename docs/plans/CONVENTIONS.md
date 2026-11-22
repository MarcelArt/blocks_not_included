# Conventions for every step

You are implementing one step of the Sim Sandbox milestone. Your step file is the task; this file holds the rules all steps share.

## Read first

- `CONTEXT.md` is the glossary. Name types, functions and tests with its terms (Cell, Element, Phase, Mass, Density, Chunk, Tick, …) and follow its _Avoid_ lists.
- `docs/adr/` holds decisions already made. Every step works inside them.

## Test-first workflow

- Work red → green, one test at a time. Use the `/tdd` skill if it is available.
- The seams and tests your step lists are already agreed with the human, so start the loop straight away. If the implementation reveals a case worth pinning at the same seam, add a test for it.
- Tests go at the seam. For library crates that means `crates/<crate>/tests/*.rs`, so tests can only touch the public interface.
- Take expected values from worked examples or literal numbers, and put the arithmetic in a comment next to the assertion.
- Tests that run Ticks check the invariants below after every Tick.
- Tests at the seam need not be bit-exact. A tolerance such as 0.01 K or 1 g is fine, as long as the comment says why.

## Invariants (simulation)

- Every Cell holds exactly one Element. A Vacuum Cell has zero Mass, and every other Cell has at least 1 g.
- A Tick never changes total Mass. The only exceptions are the ones a step names explicitly: god-tool Edits, and gas lost past the world ceiling.
- No direction is favoured: a mirror-symmetric setup stays mirror-symmetric.
- The same state and Tick count always produce the same result.

## Crate boundaries and units

- `crates/sim` and `crates/worldgen` stay Bevy-free (ADR 0003). They depend only on std plus the crates a step names. Everything that uses Bevy lives in the game crate at the repository root.
- Mass is whole grams (`u32`; sums are `u64`). Temperature is kelvin (`f32`). A Cell is a 1 m cube. A Tick is 0.2 s of simulated time.

## Dependencies

- Before adding a crate, check on crates.io that its latest version works with this workspace. Game-side crates must support Bevy 0.20.
- Use Context7 for current library documentation.
- Game steps (09 onward): read `docs/plans/BEVY-0.20-NOTES.md` before writing any Bevy code.

## When to stop and ask the human

- A rule in your step contradicts `CONTEXT.md`, an ADR, or an earlier step's tests.
- The work needs a design decision your step leaves open, and that decision would be costly to reverse.
- A new domain term emerges. Propose a glossary entry and wait for the human's answer.

## Done

A step is done when every item in its **Done when** list holds, and also:

- `cargo test --workspace` passes;
- `cargo clippy --workspace --all-targets` reports no warnings;
- the step's row in `docs/plans/README.md` reads **done**.

Finish with a short report: what you built, how many tests there are, anything deferred, and anything that surprised you.
