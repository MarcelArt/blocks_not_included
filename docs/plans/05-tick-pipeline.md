# Step 05: Tick pipeline, Edits, Settled Chunks and change tracking

> Read `docs/plans/CONVENTIONS.md` first.

**Prerequisites:** step 04 is done, so every phenomenon exists.

## Goal

This step builds the interface the game will use:

- The Tick runs its phenomena in a fixed order.
- The player changes Cells through **Edits**.
- Chunks where nothing is changing skip work (Settled Chunks).
- The game can ask which Chunks changed and read their Cells.
- Tuning values live in `SimSettings`.

## Design

- **Tick order:** heat → gas → liquid → Phase Transitions. Write this down once, as the body of `Simulation::tick`.
- **Edits.** These are god tools, so they create and destroy matter, which is the named exception to Mass conservation.
  ```rust
  pub enum Edit {
      Place { pos: CellPos, cell: Cell },          // overwrites whatever was there
      Dig { pos: CellPos },                        // the Cell becomes Vacuum
      ShiftTemperature { pos: CellPos, kelvin: f32 }, // non-Vacuum only; never below 0 K
  }
  ```
  `Simulation::apply(edit)` applies an Edit straight away; the game calls it between Ticks. `set` stays available for tests and world generation. Both wake the Chunk they touch.
- **`SimSettings`**: a `Deserialize` struct with `Default`, passed to `Simulation::set_settings`. It contains:
  - `conduction_multiplier`, default 1.0. With real values, heat takes days to cross one Cell, so this is the global speed-up.
  - `settle_threshold_k`, default 0.01.
  - `settled_chunks_enabled`, default true.
  
  The flow constants from steps 02 and 03 (evening-out fraction, spreading threshold) move here too.
- **Settled Chunks.** After a Tick, a Chunk is Settled if, during that Tick, none of its Cells changed Element or Mass and no temperature changed by more than `settle_threshold_k`. Every phenomenon skips a Settled Chunk. Two things wake it:
  - an Edit or `set` inside it;
  - any change to one of its Cells, including heat or Mass arriving across a border from an awake neighbour. Borders between an awake Chunk and a Settled one are still processed.
- **Change tracking.** `take_changed_chunks() -> Vec<ChunkPos>` returns every Chunk that had any Cell change since the last call, then clears the list. Make the following public:
  - `ChunkPos`, with `ChunkPos::of(CellPos)` and `ChunkPos::origin() -> CellPos`;
  - `CHUNK_SIZE`;
  - `chunk_cells(ChunkPos) -> Option<&[Cell]>`. Document the index order (x fastest, then z, then y) on the method.
- **`stats() -> SimStats`**, with `tick_count`, `awake_chunks`, `settled_chunks` and `total_mass_grams`.

## Seams and tests

**Seam:** `Simulation`. Put the tests in `crates/sim/tests/pipeline.rs`, in this order:

1. `Edit::Dig` turns a Cell into Vacuum, and neighbouring gas flows in over the next Ticks.
2. `Edit::Place` overwrites a Cell, and total Mass changes by exactly the difference.
3. `Edit::ShiftTemperature` warms a Cell by the given number of kelvin, and never takes it below 0 K.
4. Heat runs before Phase Transitions within a Tick: Water at 373.0 K that heat pushes past 373.15 K during a Tick has become Steam by the end of that same Tick.
5. Doubling `conduction_multiplier` doubles a single Tick's heat flow, in a regime where the per-face cap doesn't apply.
6. A Chunk where nothing happens is Settled after one Tick, as reported by `stats`.
7. An Edit in a Settled Chunk wakes it, and its effects spread: dig next to gas, and the gas flows in.
8. Heat flowing out of an awake Chunk wakes a Settled neighbour, and the heat crosses the border.
9. Settled Chunks change nothing beyond the threshold. Run a cooling hot-spot scenario with `settled_chunks_enabled` on and off; after N Ticks every Cell matches within `settle_threshold_k` and exactly in Mass.
10. `take_changed_chunks` reports exactly the Chunks that changed, then returns an empty list.

## Done when

- [ ] All ten tests exist and pass, and all earlier tests still pass.

## Scope boundary

This step ends with every loaded Chunk simulated. Limiting simulation to the Active Region comes in step 06.
