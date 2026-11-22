# Step 06: Active Region, Dormant Chunks and world limits

> Read `docs/plans/CONVENTIONS.md` first, then `docs/adr/0001-infinite-world-active-region.md`.

**Prerequisites:** step 05 is done.

## Goal

Only the Active Region around a focus point is simulated. Chunks outside it are Dormant: their time is stopped and nothing crosses their border. New Chunks come from a pluggable source. The world has a sealed floor and a ceiling that gas escapes through.

## Design

- **Chunk source.** `pub trait ChunkSource: Send { fn generate(&self, chunk: ChunkPos) -> Vec<Cell>; }`. It returns `CHUNK_SIZE³` Cells in `chunk_cells` index order. Add `Simulation::with_source(elements, Box<dyn ChunkSource>)`; `Simulation::new` uses a source that produces all-Vacuum Chunks.
- **Active Region.** A box of Chunks around the focus, with the size taken from `SimSettings` (default 4×2×4).
  - `Simulation::focus_on(pos: CellPos)` sets the focus. The default focus is the origin, so earlier tests keep their behaviour.
  - Apply **hysteresis**: walking back and forth across a Chunk border must not shift the box. The box moves only once the focus is clearly past a border; pick and document a margin, for example 4 Cells.
- **Loading.** A Chunk entering the Active Region is loaded from memory if it has been seen before, and generated from the source otherwise. `Simulation::load(chunk)` loads a Chunk without activating it, for render distance; it is Dormant.
- **Dormant Chunks.** No phenomenon touches them. No Mass or heat crosses between an active Chunk and a Dormant or unloaded one (ADR 0001). An Edit aimed at a Dormant Chunk changes its stored data, and that change takes effect once the Chunk is active.
- **World limits.** Constants `WORLD_BOTTOM_Y = -1024` and `WORLD_TOP_Y = 256` (Cell y, exclusive top); both are multiples of `CHUNK_SIZE`. Chunks outside this range are never loaded.
  - **Floor:** sealed.
  - **Ceiling:** each Tick, a gas Cell in the top layer loses a fraction of its Mass (a `SimSettings` value) out of the world. This is the second named exception to Mass conservation; count it as `mass_lost_to_space_grams` in `SimStats`.
- **`SimStats`** gains `active_chunks` and `dormant_chunks`.

### Decisions made during implementation

- **Placement and hysteresis** (`crates/sim/src/region.rs`). Along each axis the box sits where its centre is nearest the focus. It moves only once the focus is more than 4 Cells past the point where the next box over would be better centred. With the default even sizes that point runs through the middle of a Chunk, so crossing a Chunk border never moves the box. A focus-Chunk box would have left the focus as little as 1 Cell from the top of a 2-Chunk-tall region.
- **World limits** cut the box off rather than shifting it, so near the floor or ceiling the Active Region holds 16 Chunks instead of 32.
- **The sealed edge.** Phenomena see every Cell outside the Active Region as a solid that conducts no heat. Floor and ceiling are that same edge; only the ceiling lets gas out.
- **All-Vacuum Chunks aren't stored.** A generated Chunk of nothing but Vacuum counts as loaded, but takes no memory until something enters it. `chunk_cells` returns `None` for it, and `SimStats` doesn't count it. This keeps `Simulation::new` as cheap as before, and keeps step 05's Chunk counts. The source is still asked for each Chunk only once.
- **Waking.** When the Active Region moves, every Chunk that entered, and every Chunk sharing a face with one that entered or left, is woken. A Settled Chunk against the old edge may not be at rest against the new one.
- **New public items:** `ActiveRegion` (returned by `Simulation::active_region`), `WORLD_BOTTOM_Y`, `WORLD_TOP_Y`, and the settings `active_region_chunks` (written `(4, 2, 4)` in RON) and `gas_escape_fraction` (default 0.01). Gas escapes right after gas flow in each Tick.
- **`take_changed_chunks`** also lists Chunks loaded with something in them, so step 09 can send them.
- *(Approved by the human.)* Step 02's three-gas stack test sat at y = 31..=33, beyond the default region's top at y = 31, while other tests reach down to y = −4. Its column now straddles the Chunk border at y = 0 instead.

## Seams and tests

**Seam:** `Simulation`, plus `ChunkSource`. Test sources fill Chunks with a marker Element. Put the tests in `crates/sim/tests/active_region.rs`, in this order:

1. A hot spot in a Dormant Chunk keeps its exact temperature for 100 Ticks.
2. The edge is sealed: gas and heat next to the Active Region's edge never cross into the Dormant Chunk beyond it, and total Mass inside stays the same.
3. Moving the focus activates a Dormant Chunk, and it resumes exactly where it was left.
4. A never-seen Chunk entering the Active Region comes from the `ChunkSource`.
5. Moving the focus back and forth across a Chunk border keeps the same Active Region.
6. Gas in the top layer of the world leaves over time; `stats` reports exactly the Mass lost, and all other Mass is conserved.
7. Chunks below `WORLD_BOTTOM_Y` or at or above `WORLD_TOP_Y` never load, even when the focus is near the limit.

Added during implementation, at the same seam:

- Gas on the world's floor never leaves through it.
- An Edit to a Dormant Chunk is stored, and takes effect once the Chunk is active.
- A Settled Chunk left behind by the moving edge wakes, and its gas flows on into the newly active Chunk.

## Done when

- [x] All seven tests exist and pass, and all earlier tests still pass. Earlier tests rely on the default focus at the origin.

## Scope boundary

This step ends with the machinery. What the world contains comes in step 07, and following the player comes in step 11.
