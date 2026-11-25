use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::cell::{Cell, CellPos};
use crate::checkpoint::{Checkpoint, CheckpointError, ChunkCheckpoint, ChunkContents};
use crate::chunk::{ActiveChunks, CHUNK_VOLUME, Chunk, ChunkPos, fingerprint, locate};
use crate::edit::Edit;
use crate::element::{ElementError, ElementId, ElementTable};
use crate::region::{ActiveRegion, Placement, is_in_world};
use crate::settings::{SettingsError, SimSettings};
use crate::settled::TickStart;
use crate::transfer::FACES;
use crate::{gas, heat, liquid, phase_transition};

/// Simulated time covered by one Tick.
pub(crate) const TICK_SECONDS: f32 = 0.2;

/// How many threads a Tick is split across unless [`Simulation::set_threads`] says otherwise.
/// The game's Bevy task pools share the machine (`docs/plans/BEVY-0.20-NOTES.md`), so the
/// simulation keeps to a few threads of its own.
const DEFAULT_THREADS: usize = 3;

/// Where new Chunks come from: the world generator, or a test's own.
pub trait ChunkSource: Send {
    /// The Cells of the Chunk at `chunk`, which the simulation has never loaded: `CHUNK_SIZE³` of
    /// them, in the order [`Simulation::chunk_cells`] lists them.
    ///
    /// The simulation asks for each Chunk at most once, and only for Chunks within the world's
    /// vertical limits, from [`WORLD_BOTTOM_Y`](crate::WORLD_BOTTOM_Y) up to
    /// [`WORLD_TOP_Y`](crate::WORLD_TOP_Y).
    fn generate(&self, chunk: ChunkPos) -> Vec<Cell>;
}

/// The source behind [`Simulation::new`]: every Chunk is all Vacuum.
struct VacuumSource;

impl ChunkSource for VacuumSource {
    fn generate(&self, _: ChunkPos) -> Vec<Cell> {
        vec![Cell::VACUUM; CHUNK_VOLUME]
    }
}

/// A summary of the simulation's state, for the debug panel and the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimStats {
    /// How many Ticks have run.
    pub tick_count: u64,
    /// Stored Chunks in the Active Region: `awake_chunks + settled_chunks`. A Chunk that has held
    /// nothing but Vacuum since it was loaded needs no storage, and isn't counted.
    pub active_chunks: usize,
    /// Active Chunks that the next Tick simulates.
    pub awake_chunks: usize,
    /// Settled Chunks (`CONTEXT.md`), which the next Tick skips.
    pub settled_chunks: usize,
    /// Stored Dormant Chunks (`CONTEXT.md`): loaded, but outside the Active Region, so their time
    /// is stopped.
    pub dormant_chunks: usize,
    /// The Mass of every Cell in the world added together.
    pub total_mass_grams: u64,
    /// The Mass that gas has lost through the world's ceiling since the simulation started. This
    /// is the second named exception to Mass conservation, after Edits.
    pub mass_lost_to_space_grams: u64,
}

/// The whole simulated world.
///
/// Only the Active Region (`CONTEXT.md`) is simulated: a box of Chunks around the focus, which
/// [`focus_on`](Self::focus_on) moves. A Chunk entering it is loaded, from memory if it was loaded
/// before and from the [`ChunkSource`] otherwise. Chunks outside it are Dormant: their time is
/// stopped, and the Active Region's edge is sealed, so no Mass or heat crosses into them
/// (ADR 0001). A Cell in a Chunk that isn't loaded reads as Vacuum.
///
/// Between Ticks, [`checkpoint`](Self::checkpoint) captures it, and [`restore`](Self::restore)
/// builds a simulation from a checkpoint that carries on exactly as it would have.
pub struct Simulation {
    elements: ElementTable,
    settings: SimSettings,
    source: Box<dyn ChunkSource>,
    /// The stored Chunks of the Active Region, which every phenomenon works on.
    active: ActiveChunks,
    /// The stored Dormant Chunks.
    dormant: HashMap<ChunkPos, Chunk>,
    /// Chunks the source generated as nothing but Vacuum. They count as loaded without being
    /// stored until something enters them, so this may also list some that are stored by now.
    vacuum_only: HashSet<ChunkPos>,
    focus: CellPos,
    /// Where the Active Region's box sits; it follows the focus.
    placement: Placement,
    /// How many Ticks have run; seeds the flow tie-breaks.
    ticks: u64,
    /// Chunks loaded with something in them, or in which a Cell has changed, since the last call
    /// to `take_changed_chunks`.
    changed: BTreeSet<ChunkPos>,
    mass_lost_to_space_grams: u64,
    /// How many threads a Tick is split across.
    threads: usize,
    /// The threads themselves, started by the first Tick.
    pool: Option<rayon::ThreadPool>,
}

impl Simulation {
    /// A world of nothing but Vacuum.
    pub fn new(elements: ElementTable) -> Self {
        Self::with_source(elements, Box::new(VacuumSource))
    }

    /// A world whose Chunks come from `source`. The Active Region around the default focus, the
    /// origin, is loaded straight away.
    pub fn with_source(elements: ElementTable, source: Box<dyn ChunkSource>) -> Self {
        let settings = SimSettings::default();
        let focus = CellPos::new(0, 0, 0);
        let placement = Placement::around(focus, settings.active_region_chunks);
        let mut sim = Self {
            elements,
            settings,
            source,
            active: ActiveChunks::new(ActiveRegion::EMPTY),
            dormant: HashMap::new(),
            vacuum_only: HashSet::new(),
            focus,
            placement,
            ticks: 0,
            changed: BTreeSet::new(),
            mass_lost_to_space_grams: 0,
            threads: DEFAULT_THREADS,
            pool: None,
        };
        sim.move_region(placement.region());
        sim
    }

    /// The simulation `checkpoint` captured, restored with `elements` and `settings` as its rules
    /// and `source` for every Chunk the checkpoint leaves to it. Given the rules and the source
    /// the checkpoint was captured with, it carries on exactly as the captured simulation would
    /// have. Every stored Chunk counts as changed, for [`take_changed_chunks`](Self::take_changed_chunks).
    ///
    /// Dormant Chunks the source can regenerate aren't in a checkpoint, so they load from the
    /// source when they are needed. Until then, they are missing from
    /// [`stats`](Self::stats)' Dormant Chunks and total Mass, and their Cells read as Vacuum.
    ///
    /// `elements` must hold every Element the checkpoint names, at the same position, and may
    /// append more, as [`replace_elements`](Self::replace_elements) allows. Like
    /// [`new`](Self::new), this doesn't [validate](ElementTable::validate) the table.
    ///
    /// Rejects a checkpoint that doesn't fit the rules or the source, or that breaks the Cell
    /// invariants, before anything is built. The source is asked for the Chunks of the Active
    /// Region the checkpoint leaves to it, as [`with_source`](Self::with_source) would.
    pub fn restore(
        elements: ElementTable,
        settings: SimSettings,
        source: Box<dyn ChunkSource>,
        checkpoint: Checkpoint,
    ) -> Result<Self, CheckpointError> {
        settings.validate().map_err(CheckpointError::Settings)?;
        elements
            .check_holds(checkpoint.element_positions())
            .map_err(CheckpointError::Elements)?;
        if checkpoint.placement_chunks != settings.active_region_chunks {
            return Err(CheckpointError::RegionSize {
                checkpoint: checkpoint.placement_chunks,
                settings: settings.active_region_chunks,
            });
        }
        let placement = Placement::at(checkpoint.placement_min, checkpoint.placement_chunks);
        if placement.follow(checkpoint.focus) != placement {
            return Err(CheckpointError::RegionAwayFromFocus {
                focus: checkpoint.focus,
                placement_min: checkpoint.placement_min,
            });
        }
        let region = placement.region();
        checkpoint.validate_chunks(region)?;

        let mut sim = Self {
            elements,
            settings,
            source,
            active: ActiveChunks::new(region),
            dormant: HashMap::new(),
            vacuum_only: HashSet::new(),
            focus: checkpoint.focus,
            placement,
            ticks: checkpoint.tick_count,
            changed: BTreeSet::new(),
            mass_lost_to_space_grams: checkpoint.mass_lost_to_space_grams,
            threads: DEFAULT_THREADS,
            pool: None,
        };
        for (pos, listed) in checkpoint.chunks {
            let mut chunk = match listed.contents {
                ChunkContents::Generated => sim
                    .generate_cells(pos)
                    .map_or_else(Chunk::new, Chunk::generated),
                ChunkContents::Vacuum => Chunk::restored(vec![Cell::VACUUM; CHUNK_VOLUME]),
                ChunkContents::Cells(cells) => Chunk::restored(cells),
            };
            chunk.settled = listed.settled;
            sim.changed.insert(pos);
            if region.contains(pos) {
                sim.active.insert(pos, chunk);
            } else {
                sim.dormant.insert(pos, chunk);
            }
        }
        for pos in region.chunks() {
            if !sim.active.contains_key(&pos) {
                if sim.generate_cells(pos).is_some() {
                    return Err(CheckpointError::MissingChunk { chunk: pos });
                }
                sim.vacuum_only.insert(pos);
            }
        }
        Ok(sim)
    }

    /// Captures the simulation as it stands, between Ticks, for [`restore`](Self::restore).
    ///
    /// It holds the Cells of every Chunk that has changed since it was loaded, by Ticks or by
    /// Edits, active or Dormant, unless it has come back to exactly what the source generates. A
    /// Chunk that holds nothing but Vacuum, where the source generates something else, holds no
    /// Cells but [`ChunkContents::Vacuum`]. It holds no Cells of any other Chunk.
    ///
    /// [`take_changed_chunks`](Self::take_changed_chunks) doesn't affect it.
    pub fn checkpoint(&self) -> Checkpoint {
        let mut chunks = BTreeMap::new();
        for (&pos, chunk) in self.active.iter() {
            let listed = ChunkCheckpoint {
                contents: self.contents(pos, chunk),
                settled: chunk.settled,
            };
            chunks.insert(pos, listed);
        }
        for (&pos, chunk) in &self.dormant {
            let contents = self.contents(pos, chunk);
            if contents != ChunkContents::Generated {
                // Whether a Dormant Chunk was Settled doesn't matter: it wakes as it enters the
                // Active Region.
                let listed = ChunkCheckpoint {
                    contents,
                    settled: false,
                };
                chunks.insert(pos, listed);
            }
        }
        Checkpoint {
            element_names: self
                .elements
                .iter()
                .map(|(_, element)| element.name.clone())
                .collect(),
            focus: self.focus,
            placement_min: self.placement.min(),
            placement_chunks: self.placement.size(),
            tick_count: self.ticks,
            mass_lost_to_space_grams: self.mass_lost_to_space_grams,
            chunks,
        }
    }

    /// The Element table the simulation runs with.
    pub fn elements(&self) -> &ElementTable {
        &self.elements
    }

    /// Replaces the tuning values, which take effect from the next Tick, and wakes every Chunk,
    /// since a Chunk at rest under the old values may not be under the new ones. A new Active
    /// Region size takes effect at once, with the box best centred on the focus.
    ///
    /// Rejects settings that fail [`SimSettings::validate`], keeping the previous ones.
    pub fn set_settings(&mut self, settings: SimSettings) -> Result<(), SettingsError> {
        settings.validate()?;
        let resized = settings.active_region_chunks != self.settings.active_region_chunks;
        self.settings = settings;
        for chunk in self.active.values_mut() {
            chunk.settled = false;
        }
        if resized {
            self.placement = Placement::around(self.focus, self.settings.active_region_chunks);
            self.move_region(self.placement.region());
        }
        Ok(())
    }

    pub fn settings(&self) -> &SimSettings {
        &self.settings
    }

    /// Replaces the Element table, which takes effect from the next Tick, and wakes every Chunk,
    /// since a Chunk at rest under the old properties may not be under the new ones.
    ///
    /// Cells store their Element as an [`ElementId`], its position in the table. So `elements`
    /// must hold every Element of the current table, under the same name and at the same
    /// position; it may change their properties, and append new Elements after them. Otherwise it
    /// is rejected with [`ElementError::MissingElement`] or [`ElementError::ElementMoved`], and
    /// the current table stays.
    ///
    /// Like [`new`](Self::new), this doesn't [validate](ElementTable::validate) the table.
    pub fn replace_elements(&mut self, elements: ElementTable) -> Result<(), ElementError> {
        self.elements.check_replacement(&elements)?;
        self.elements = elements;
        for chunk in self.active.values_mut() {
            chunk.settled = false;
        }
        Ok(())
    }

    /// Sets how many threads each Tick is split across, from the next Tick on; 0 counts as 1.
    /// The default is 3.
    ///
    /// A Tick gives exactly the same result whatever the number of threads.
    pub fn set_threads(&mut self, threads: usize) {
        let threads = threads.max(1);
        if threads != self.threads {
            self.threads = threads;
            self.pool = None;
        }
    }

    /// How many threads each Tick is split across.
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Moves the focus of the Active Region to `pos`, such as the player's Cell. The Active
    /// Region follows with hysteresis (see `region.rs`), so walking back and forth doesn't shift
    /// it. Chunks entering it are loaded and woken; Chunks leaving it become Dormant.
    pub fn focus_on(&mut self, pos: CellPos) {
        self.focus = pos;
        self.placement = self.placement.follow(pos);
        self.move_region(self.placement.region());
    }

    /// The focus of the Active Region, as [`focus_on`](Self::focus_on) last moved it. It starts
    /// at the origin.
    pub fn focus(&self) -> CellPos {
        self.focus
    }

    pub fn active_region(&self) -> ActiveRegion {
        self.active.region
    }

    /// Loads the Chunk at `chunk` without making it active, for render distance: outside the
    /// Active Region it is Dormant. Does nothing if it is loaded already, or if it lies outside
    /// the world's vertical limits.
    pub fn load(&mut self, chunk: ChunkPos) {
        if is_in_world(chunk)
            && !self.is_loaded(chunk)
            && let Some(loaded) = self.generate(chunk)
        {
            // Every Chunk of the Active Region is loaded already, so this one is Dormant.
            self.dormant.insert(chunk, loaded);
        }
    }

    /// Overwrites the Cell at `pos`, for tests and world generation, and wakes its Chunk.
    ///
    /// A Chunk that was never loaded is loaded first. In a Dormant Chunk the change is stored, and
    /// takes effect once the Chunk is active. Outside the world's vertical limits, where no Chunk
    /// is ever loaded, nothing happens.
    pub fn set(&mut self, pos: CellPos, cell: Cell) {
        let (chunk_pos, index) = locate(pos);
        let Some(chunk) = self.stored_mut(chunk_pos) else {
            return;
        };
        let changed = chunk.cells[index] != cell;
        chunk.write(index, cell);
        chunk.settled = false;
        if changed {
            self.changed.insert(chunk_pos);
        }
    }

    /// Applies `edit` straight away. The game calls this between Ticks. Like
    /// [`set`](Self::set), an Edit to a Dormant Chunk takes effect once the Chunk is active.
    pub fn apply(&mut self, edit: Edit) {
        let pos = edit.pos();
        let cell = edit.result(self.get(pos));
        self.set(pos, cell);
    }

    /// The Cell at `pos`. A Cell in a Chunk that isn't loaded reads as Vacuum.
    pub fn get(&self, pos: CellPos) -> Cell {
        let (chunk, index) = locate(pos);
        self.stored(chunk)
            .map_or(Cell::VACUUM, |chunk| chunk.cells[index])
    }

    /// The Cells of the loaded Chunk at `chunk`, or `None` if it isn't loaded or has held nothing
    /// but Vacuum since it was.
    ///
    /// There are `CHUNK_SIZE³` of them, x fastest, then z, then y: the Cell at
    /// `chunk.origin()` offset by `(x, y, z)` is at index `(y × CHUNK_SIZE + z) × CHUNK_SIZE + x`.
    pub fn chunk_cells(&self, chunk: ChunkPos) -> Option<&[Cell]> {
        self.stored(chunk).map(|chunk| &*chunk.cells)
    }

    /// The Mass of every Cell in the world added together.
    pub fn total_mass_grams(&self) -> u64 {
        self.active
            .values()
            .chain(self.dormant.values())
            .flat_map(|chunk| chunk.cells.iter())
            .map(|cell| u64::from(cell.mass_grams))
            .sum()
    }

    /// Every Chunk loaded with something in it, or in which any Cell has changed, since the last
    /// call, sorted by x, then y, then z. The list starts empty again afterwards.
    pub fn take_changed_chunks(&mut self) -> Vec<ChunkPos> {
        std::mem::take(&mut self.changed).into_iter().collect()
    }

    pub fn stats(&self) -> SimStats {
        let settled_chunks = self.active.values().filter(|chunk| chunk.settled).count();
        SimStats {
            tick_count: self.ticks,
            active_chunks: self.active.len(),
            awake_chunks: self.active.len() - settled_chunks,
            settled_chunks,
            dormant_chunks: self.dormant.len(),
            total_mass_grams: self.total_mass_grams(),
            mass_lost_to_space_grams: self.mass_lost_to_space_grams,
        }
    }

    /// Advances the Active Region by one Tick: heat, then gas, with gas in the world's top layer
    /// escaping to space, then liquid, then Phase Transitions. Dormant Chunks don't change.
    pub fn tick(&mut self) {
        let threads = self.threads;
        let pool = self.pool.get_or_insert_with(|| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .thread_name(|i| format!("sim-tick-{i}"))
                .build()
                .expect("the simulation's threads start")
        });
        let chunks = &mut self.active;
        let (elements, settings, ticks) = (&self.elements, &self.settings, self.ticks);
        let escaped = pool.install(|| {
            let start = TickStart::record(chunks, settings);
            heat::conduct(chunks, elements, settings.conduction_multiplier);
            gas::flow(chunks, elements, ticks, settings);
            let escaped = gas::escape_to_space(chunks, elements, settings.gas_escape_fraction);
            liquid::flow(chunks, elements, ticks, settings);
            phase_transition::transform(chunks, elements);
            start.settle(chunks, settings);
            escaped
        });
        self.mass_lost_to_space_grams += escaped;
        self.changed.extend(
            chunks
                .iter()
                .filter(|(_, chunk)| chunk.touched)
                .map(|(&pos, _)| pos),
        );
        self.ticks += 1;
    }

    /// Makes `region` the Active Region. Chunks leaving it become Dormant, and Chunks entering it
    /// are loaded: from memory if they were loaded before, and from the source otherwise.
    ///
    /// A Chunk at rest against the old edge may not be at rest against the new one, and a Chunk
    /// that was Settled when it left may face new neighbours. So every Chunk that entered, or
    /// that shares a face with a Chunk that entered or left, is woken.
    fn move_region(&mut self, region: ActiveRegion) {
        let old = self.active.region;
        if region == old {
            return;
        }
        let leaving: Vec<ChunkPos> = self
            .active
            .keys()
            .copied()
            .filter(|&pos| !region.contains(pos))
            .collect();
        for pos in leaving {
            let chunk = self.active.remove(&pos).expect("listed as active");
            self.dormant.insert(pos, chunk);
        }
        self.active.region = region;
        for pos in region.chunks().filter(|&pos| !old.contains(pos)) {
            let chunk = match self.dormant.remove(&pos) {
                Some(chunk) => Some(chunk),
                None if self.vacuum_only.contains(&pos) => None,
                None => self.generate(pos),
            };
            if let Some(chunk) = chunk {
                self.active.insert(pos, chunk);
            }
        }
        let moved = |pos: ChunkPos| old.contains(pos) != region.contains(pos);
        for (&pos, chunk) in self.active.iter_mut() {
            if moved(pos) || FACES.into_iter().any(|face| moved(pos.offset(face))) {
                chunk.settled = false;
            }
        }
    }

    /// Whether the Chunk at `chunk` is loaded. Every Chunk of the Active Region is.
    fn is_loaded(&self, chunk: ChunkPos) -> bool {
        self.active.region.contains(chunk)
            || self.dormant.contains_key(&chunk)
            || self.vacuum_only.contains(&chunk)
    }

    /// Generates the never-loaded Chunk at `chunk` with the source. A Chunk of nothing but
    /// Vacuum isn't stored, so it gives `None`. Any other is listed as changed, so it gets drawn.
    fn generate(&mut self, chunk: ChunkPos) -> Option<Chunk> {
        let Some(cells) = self.generate_cells(chunk) else {
            self.vacuum_only.insert(chunk);
            return None;
        };
        self.changed.insert(chunk);
        Some(Chunk::generated(cells))
    }

    /// The Cells the source generates for the Chunk at `chunk`, or `None` if they are nothing but
    /// Vacuum.
    fn generate_cells(&self, chunk: ChunkPos) -> Option<Vec<Cell>> {
        let cells = self.source.generate(chunk);
        assert_eq!(
            cells.len(),
            CHUNK_VOLUME,
            "a ChunkSource must give CHUNK_SIZE³ Cells, but it gave {} for {chunk:?}",
            cells.len()
        );
        let vacuum = cells.iter().all(|cell| cell.element == ElementId::VACUUM);
        (!vacuum).then_some(cells)
    }

    /// What a checkpoint holds of `chunk`, the stored Chunk at `pos`.
    fn contents(&self, pos: ChunkPos, chunk: &Chunk) -> ChunkContents {
        if !chunk.modified || self.is_as_generated(pos, chunk) {
            ChunkContents::Generated
        } else if chunk
            .cells
            .iter()
            .all(|cell| cell.is_identical(&Cell::VACUUM))
        {
            ChunkContents::Vacuum
        } else {
            ChunkContents::Cells(chunk.cells.to_vec())
        }
    }

    /// Whether `chunk`, the stored Chunk at `pos`, holds exactly what the source generates for
    /// it, as it was stored when loaded. Only a fingerprint match is confirmed by generating the
    /// Chunk again, so a Chunk that has changed rarely costs more than the fingerprint.
    fn is_as_generated(&self, pos: ChunkPos, chunk: &Chunk) -> bool {
        if chunk.baseline != Some(fingerprint(&chunk.cells)) {
            return false;
        }
        let generated = self
            .generate_cells(pos)
            .unwrap_or_else(|| vec![Cell::VACUUM; CHUNK_VOLUME]);
        generated
            .iter()
            .zip(&chunk.cells)
            .all(|(generated, cell)| generated.is_identical(cell))
    }

    /// The stored Chunk at `chunk`, if it is loaded.
    fn stored(&self, chunk: ChunkPos) -> Option<&Chunk> {
        self.active.get(&chunk).or_else(|| self.dormant.get(&chunk))
    }

    /// The stored Chunk at `chunk`, loaded first if it never was, and stored if it held nothing
    /// but Vacuum. `None` outside the world's vertical limits.
    fn stored_mut(&mut self, chunk: ChunkPos) -> Option<&mut Chunk> {
        if !is_in_world(chunk) {
            return None;
        }
        self.load(chunk);
        let chunks = if self.active.region.contains(chunk) {
            &mut *self.active
        } else {
            &mut self.dormant
        };
        Some(chunks.entry(chunk).or_insert_with(Chunk::new))
    }
}
