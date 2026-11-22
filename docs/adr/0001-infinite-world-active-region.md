# Infinite world, simulated only inside the Active Region

The world is infinite and procedurally generated, but only the Active Region is simulated. The Active Region is a box of roughly 1M Cells (4×2×4 Chunks of 32³) around the player. Its edge is sealed and insulated, so no Mass or heat crosses into a Dormant Chunk, and time in a Dormant Chunk is stopped. This keeps the infinite world of a Minecraft-style game while putting a bound on simulation cost. A 3D world has roughly 100× the Cells of Oxygen Not Included's 2D map, so simulating all of it is not an option.

## Considered Options

- **Bounded world, fully simulated** (like the ONI asteroid). The same rules apply everywhere, but the game loses infinite exploration.
- **Reservoir edge**: the dormant side acts as a fixed-state source or sink. The edge looks natural, but matter is created and destroyed there, and players could farm it.
- **One-way inbox edge**: Dormant Chunks absorb what arrives but don't spread it. Matter is conserved, but edge Cells accumulate absurd pressure.
- **Catch-up simulation** when a Chunk wakes: approximate and a rabbit hole.

## Consequences

- Gas can pile up against the invisible edge. This is acceptable because the edge is out of the player's sight.
- Anything inside a Dormant Chunk does nothing. Remote automation would need an explicit mechanic for keeping Chunks active.
- The world generator must be deterministic per Chunk position, because unchanged Chunks are regenerated from the seed.
- Generation must produce settled Chunks, with each Element chosen to match the local temperature. Otherwise every newly activated Chunk churns.
- Chunks are cubic and the world has a deep but fixed vertical range, so the Active Region can stay a small box rather than full-height columns.
