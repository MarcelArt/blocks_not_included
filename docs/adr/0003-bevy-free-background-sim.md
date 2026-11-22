# The simulation is a Bevy-free core running on a background thread

All Cell data belongs to a simulation written in plain Rust, with no Bevy types. It steps at a fixed rate of about 5 Ticks per second on a background thread. The game renders from the most recent completed Tick, and player edits are queued and applied between Ticks. One Tick over about 1M Cells is expected to take longer than a 16 ms frame, so running it inside a frame would cause hitches. Moving ownership of Cell data out of the game later would mean a restructure, so we decided it up front.

## Considered Options

- **CPU, inside Bevy's `FixedUpdate`**: simple, but a slow Tick stalls the frame it runs in.
- **GPU compute**: Bevy dispatches compute work once per rendered frame rather than once per Tick. Reading results back to the CPU takes about 3 frames. Bevy also changed these APIs in both 0.19 and 0.20.

## Consequences

- Cells are never ECS entities. Chunk entities exist only for rendering.
- Anything on the game side that reads simulation state sees state up to one Tick old. That includes rendering, the crosshair readout and, later, breathing.
- The simulation can be unit-tested and benchmarked without Bevy.
