//! World saves on disk and the catalog of Worlds (`docs/plans/17-world-save-storage.md`): a
//! `WorldStore` in a temporary folder, with no Bevy app.
//!
//! Some tests reach into a save file to damage it or to fake one from another version. They rely
//! on the layout `src/saves/format.rs` documents: an 8-byte magic, the format revision as a
//! little-endian u32, the header's length as another, then the header as RON text.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bevy::math::Vec3;
use blocks_not_included::player::movement::{Body, MoveMode};
use blocks_not_included::saves::{
    GameMode, Incompatibility, PlayerState, RulesPolicy, StoreError, WorldId, WorldSave, WorldStore,
};
use blocks_not_included::sim_bridge::{Generation, SimCapture};
use sim::{
    CHUNK_SIZE, Cell, CellPos, Checkpoint, ChunkCheckpoint, ChunkContents, ChunkPos, Element,
    ElementTable, Phase, SimSettings, Simulation,
};

const SHIPPED_ELEMENTS: &str = include_str!("../assets/elements.ron");
const SHIPPED_SETTINGS: &str = include_str!("../assets/sim_settings.ron");
const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;
const SEED: u64 = 2026;
/// Far longer than anything here should take.
const TIMEOUT: Duration = Duration::from_secs(10);

/// A folder of its own under the system's temporary folder, removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "bni-world-store-{name}-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temporary folder");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The names of the files in `dir`, sorted.
fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("readable")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .into_string()
                .expect("UTF-8")
        })
        .collect();
    names.sort();
    names
}

/// `ms` milliseconds after the Unix epoch. Saves keep last-played times to the millisecond.
fn at(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn shipped_elements() -> ElementTable {
    ElementTable::from_ron(SHIPPED_ELEMENTS).expect("the shipped Elements are valid")
}

fn shipped_settings() -> SimSettings {
    ron::from_str(SHIPPED_SETTINGS).expect("the shipped settings are valid")
}

/// A store for a development build, which restores each World's own rules.
fn dev_store(dir: &Path) -> WorldStore {
    WorldStore::open(dir, RulesPolicy::AsSaved).expect("the store opens")
}

/// A store for a release build, which ships the files in `assets/`.
fn release_store(dir: &Path) -> WorldStore {
    let rules = RulesPolicy::Shipped {
        elements: shipped_elements(),
        settings: shipped_settings(),
    };
    WorldStore::open(dir, rules).expect("the store opens")
}

/// The name of every Element of `elements`, in id order from Vacuum, as a checkpoint lists them.
fn names(elements: &ElementTable) -> Vec<String> {
    elements
        .iter()
        .map(|(_, element)| element.name.clone())
        .collect()
}

fn generated(settled: bool) -> ChunkCheckpoint {
    ChunkCheckpoint {
        contents: ChunkContents::Generated,
        settled,
    }
}

/// A World just created from `seed` under the shipped rules: every Chunk of the default Active
/// Region around the origin still as generated, and a player standing at the origin.
fn fresh_save(seed: u64) -> WorldSave {
    let elements = shipped_elements();
    let mut chunks = BTreeMap::new();
    for x in -2..2 {
        for y in -1..1 {
            for z in -2..2 {
                chunks.insert(ChunkPos::new(x, y, z), generated(false));
            }
        }
    }
    let checkpoint = Checkpoint {
        element_names: names(&elements),
        focus: CellPos::new(0, 3, 0),
        placement_min: ChunkPos::new(-2, -1, -2),
        placement_chunks: [4, 2, 4],
        tick_count: 0,
        mass_lost_to_space_grams: 0,
        chunks,
    };
    WorldSave {
        capture: SimCapture {
            checkpoint,
            elements: elements.clone(),
            settings: shipped_settings(),
            generation: Some(Generation { seed, elements }),
        },
        player: PlayerState {
            body: Body {
                grounded: true,
                ..Body::standing_at(Vec3::new(0.5, 3.0, 0.5))
            },
            yaw: 0.25,
            pitch: 0.0,
        },
    }
}

/// Temperatures that only survive bit for bit: −0 K, a subnormal, the f32 just above room
/// temperature, a NaN with a payload, and infinity. The simulation keeps temperatures exactly
/// as they are, so a save must too.
fn awkward_temperatures() -> [f32; 6] {
    [
        -0.0,
        f32::from_bits(1),
        f32::from_bits(293.15f32.to_bits() + 1),
        f32::from_bits(0x7fc0_1234),
        f32::INFINITY,
        1_473.987_5,
    ]
}

/// The World of [`fresh_save`] after play: Ticks have run, gas has escaped, a Chunk holds Cells
/// of Water and Granite at awkward temperatures, another has been dug out to Vacuum, a Dormant
/// Chunk far beyond view has changed, and the player is flying away fast, looking down.
fn changed_save(seed: u64) -> WorldSave {
    let mut save = fresh_save(seed);
    let elements = &save.capture.elements;
    let water = elements.id_of("Water").expect("shipped");
    let granite = elements.id_of("Granite").expect("shipped");
    let temperatures = awkward_temperatures();
    let cells = |offset: usize| -> Vec<Cell> {
        (0..CHUNK_VOLUME)
            .map(|i| Cell {
                element: if (i + offset).is_multiple_of(3) {
                    water
                } else {
                    granite
                },
                mass_grams: 1 + (i as u32 * 7_919) % 3_000_000,
                temperature_k: temperatures[(i + offset) % temperatures.len()],
            })
            .collect()
    };
    let checkpoint = &mut save.capture.checkpoint;
    checkpoint.tick_count = 12_345;
    checkpoint.mass_lost_to_space_grams = 987_654_321;
    checkpoint.focus = CellPos::new(1, 3, -1);
    let active = |contents| ChunkCheckpoint {
        contents,
        settled: true,
    };
    checkpoint.chunks.insert(
        ChunkPos::new(0, -1, 0),
        active(ChunkContents::Cells(cells(0))),
    );
    checkpoint
        .chunks
        .insert(ChunkPos::new(1, -1, 0), active(ChunkContents::Vacuum));
    checkpoint
        .chunks
        .insert(ChunkPos::new(-1, 0, 1), generated(true));
    checkpoint.chunks.insert(
        ChunkPos::new(400, -3, -250),
        ChunkCheckpoint {
            contents: ChunkContents::Cells(cells(1)),
            settled: false,
        },
    );
    save.player = PlayerState {
        body: Body {
            feet: Vec3::new(1.5, 3.25, -0.75),
            velocity: Vec3::new(12.0, -12.0, 0.0),
            mode: MoveMode::Fly,
            grounded: false,
        },
        yaw: -2.75,
        pitch: -1.2,
    };
    save
}

/// A Chunk of a checkpoint: its position, whether it is Settled, the kind of its contents
/// (Generated, Vacuum or Cells), and for Cells, each one's Element index, Mass and temperature
/// bits.
type ChunkBits = (ChunkPos, bool, u8, Vec<(u16, u32, u32)>);

/// Every Cell of a checkpoint's Chunks, down to the bits of each temperature: `==` would call a
/// NaN temperature different from itself, and −0 K the same as 0 K.
fn cell_bits(checkpoint: &Checkpoint) -> Vec<ChunkBits> {
    checkpoint
        .chunks
        .iter()
        .map(|(&pos, chunk)| {
            let (kind, cells) = match &chunk.contents {
                ChunkContents::Generated => (0, Vec::new()),
                ChunkContents::Vacuum => (1, Vec::new()),
                ChunkContents::Cells(cells) => (
                    2,
                    cells
                        .iter()
                        .map(|cell| {
                            let bits = cell.temperature_k.to_bits();
                            (cell.element.index(), cell.mass_grams, bits)
                        })
                        .collect(),
                ),
            };
            (pos, chunk.settled, kind, cells)
        })
        .collect()
}

/// Asserts that `loaded` is exactly `saved`, every Cell to the bit.
#[track_caller]
fn assert_same(loaded: &WorldSave, saved: &WorldSave) {
    // Compared without `assert_eq!`, which would print every Cell.
    assert!(
        cell_bits(&loaded.capture.checkpoint) == cell_bits(&saved.capture.checkpoint),
        "the Chunks differ"
    );
    let without_chunks = |save: &WorldSave| {
        let mut save = save.clone();
        save.capture.checkpoint.chunks.clear();
        save
    };
    assert_eq!(without_chunks(loaded), without_chunks(saved));
}

/// The ids of the Worlds the catalog lists, in its order.
fn listed_ids(store: &WorldStore) -> Vec<WorldId> {
    store
        .list()
        .expect("listed")
        .into_iter()
        .map(|entry| entry.id)
        .collect()
}

/// A file's bytes and modification time, to check that reading it changed neither.
fn file_state(path: &Path) -> (Vec<u8>, SystemTime) {
    let modified = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .expect("metadata");
    (fs::read(path).expect("readable"), modified)
}

/// `bytes` with `from` replaced by `to` in the header, and the header's length corrected.
fn patch_header(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let header = std::str::from_utf8(&bytes[16..16 + length]).expect("RON text");
    assert!(header.contains(from), "{from:?} isn't in {header}");
    let header = header.replacen(from, to, 1);
    let mut patched = bytes[..12].to_vec();
    patched.extend_from_slice(&(header.len() as u32).to_le_bytes());
    patched.extend_from_slice(header.as_bytes());
    patched.extend_from_slice(&bytes[16 + length..]);
    patched
}

// Seam 1: independent Worlds, names, rename and delete.

#[test]
fn two_worlds_from_the_same_name_and_seed_are_changed_renamed_and_deleted_independently() {
    let dir = TempDir::new("independent");
    let store = dev_store(dir.path());
    let fresh = fresh_save(SEED);
    let first = store.create("Twins", &fresh, at(1_000)).expect("created");
    let second = store.create("Twins", &fresh, at(1_000)).expect("created");
    assert_ne!(first.id, second.id);

    let changed = changed_save(SEED);
    let saved = store
        .save(store.ticket(&first.id), &changed, at(2_000))
        .expect("saved");
    let renamed = store.rename(&first.id, "  Left twin ").expect("renamed");

    assert_eq!(saved.last_played, at(2_000));
    // Renaming changes the name alone: the last-played time and the Cells stay.
    assert_eq!(renamed.name, "Left twin");
    assert_eq!(renamed.last_played, at(2_000));
    let first_loaded = store.load(&first.id).expect("loaded");
    assert_eq!(first_loaded.summary, renamed);
    assert_same(&first_loaded.save, &changed);

    store.delete(&first.id).expect("deleted");

    assert!(
        matches!(store.load(&first.id), Err(StoreError::NotFound(id)) if id == first.id),
        "a deleted World is gone"
    );
    assert!(matches!(
        store.rename(&first.id, "Ghost"),
        Err(StoreError::NotFound(_))
    ));
    assert!(matches!(
        store.delete(&first.id),
        Err(StoreError::NotFound(_))
    ));
    assert_eq!(listed_ids(&store), [second.id]);
    // The other World never changed.
    let second_loaded = dev_store(dir.path()).load(&second.id).expect("loaded");
    assert_eq!(second_loaded.summary, second);
    assert_eq!(second_loaded.summary.name, "Twins");
    assert_eq!(second_loaded.summary.seed, SEED);
    assert_eq!(second_loaded.summary.last_played, at(1_000));
    assert_eq!(second_loaded.summary.mode, GameMode::Sandbox);
    assert_same(&second_loaded.save, &fresh);
}

#[test]
fn a_name_is_trimmed_must_not_be_blank_and_never_chooses_where_a_world_is_stored() {
    let dir = TempDir::new("names");
    let saves = dir.path().join("saves");
    let store = dev_store(&saves);
    let fresh = fresh_save(SEED);

    for blank in ["", "   ", "\t\n "] {
        assert!(matches!(
            store.create(blank, &fresh, at(0)),
            Err(StoreError::EmptyName)
        ));
    }
    assert!(files_in(&saves).is_empty(), "a blank name wrote a file");

    let hostile = [
        "../escaped",
        "..",
        "/etc/passwd",
        "C:\\Windows\\win.ini",
        "nested/folder\\name",
        "CON",
        "nul.world",
        " \"quoted\" ) , name: \"injected\" ",
        "line\nbreak",
    ];
    for name in hostile {
        let created = store.create(name, &fresh, at(0)).expect("created");

        assert_eq!(created.name, name.trim());
        let path = store.path_of(&created.id);
        assert_eq!(path.parent(), Some(saves.as_path()));
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(format!("{}.world", created.id).as_str())
        );
        let loaded = store.load(&created.id).expect("loaded");
        assert_eq!(loaded.summary.name, name.trim());
    }
    // Every World is a file in the saves folder, and nothing was written beside it.
    assert_eq!(files_in(dir.path()), ["saves"]);
    assert_eq!(files_in(&saves).len(), hostile.len());

    let id = listed_ids(&store)[0];
    assert!(matches!(
        store.rename(&id, " \t"),
        Err(StoreError::EmptyName)
    ));

    // An id is 16 lowercase hexadecimal digits, and nothing else reads as one.
    assert_eq!(WorldId::parse(&id.to_string()), Some(id));
    for text in [
        "",
        "../0123456789abcd",
        "0123456789ABCDEF",
        "0123456789abcde",
        "0123456789abcdef0",
        "0123456789abcdeg",
        "+123456789abcdef",
    ] {
        assert_eq!(WorldId::parse(text), None, "{text:?}");
    }
}

// Seam 2: what a save holds, and which rules it loads under.

#[test]
fn fresh_and_changed_saves_load_into_the_same_values() {
    let dir = TempDir::new("round-trip");
    let store = dev_store(dir.path());
    let fresh = fresh_save(SEED);
    let changed = changed_save(SEED);

    let created = store.create("Round trip", &fresh, at(10)).expect("created");
    let fresh_loaded = dev_store(dir.path()).load(&created.id).expect("loaded");
    store
        .save(store.ticket(&created.id), &changed, at(20))
        .expect("saved");
    let changed_loaded = dev_store(dir.path()).load(&created.id).expect("loaded");

    assert_same(&fresh_loaded.save, &fresh);
    assert_same(&changed_loaded.save, &changed);
    // The flying player, every Cell of the dug-out Chunk and the Dormant one.
    let player = changed_loaded.save.player;
    assert_eq!(player.body.mode, MoveMode::Fly);
    assert_eq!(player.body.velocity, Vec3::new(12.0, -12.0, 0.0));
    let chunks = &changed_loaded.save.capture.checkpoint.chunks;
    assert_eq!(
        chunks[&ChunkPos::new(1, -1, 0)].contents,
        ChunkContents::Vacuum
    );
    assert!(matches!(
        chunks[&ChunkPos::new(400, -3, -250)].contents,
        ChunkContents::Cells(_)
    ));
}

#[test]
fn a_saved_simulation_restores_from_its_loaded_save() {
    let elements = shipped_elements();
    let settings = shipped_settings();
    let generation = Generation {
        seed: SEED,
        elements: elements.clone(),
    };
    let generator = || {
        Box::new(
            generation
                .generator()
                .expect("the shipped Elements suffice"),
        )
    };
    let mut sim = Simulation::with_source(elements.clone(), generator());
    sim.set_settings(settings.clone()).expect("valid");
    let water = elements.id_of("Water").expect("shipped");
    sim.apply(sim::Edit::Place {
        pos: CellPos::new(3, 40, 3),
        cell: Cell {
            element: water,
            mass_grams: 1_000_000,
            temperature_k: 300.0,
        },
    });
    sim.tick();
    sim.tick();
    // Dig out a whole Chunk below the surface, which the generator fills with ground.
    let dug = ChunkPos::new(-1, -1, -1);
    let origin = dug.origin();
    for dy in 0..CHUNK_SIZE as i32 {
        for dz in 0..CHUNK_SIZE as i32 {
            for dx in 0..CHUNK_SIZE as i32 {
                let pos = CellPos::new(origin.x + dx, origin.y + dy, origin.z + dz);
                sim.set(pos, Cell::VACUUM);
            }
        }
    }
    let checkpoint = sim.checkpoint();
    assert_eq!(checkpoint.chunks[&dug].contents, ChunkContents::Vacuum);
    let save = WorldSave {
        capture: SimCapture {
            checkpoint: checkpoint.clone(),
            elements,
            settings,
            generation: Some(generation),
        },
        player: fresh_save(SEED).player,
    };
    let dir = TempDir::new("simulation");
    let store = release_store(dir.path());
    let id = store.create("Played", &save, at(1)).expect("created").id;

    let loaded = release_store(dir.path()).load(&id).expect("loaded").save;
    assert_same(&loaded, &save);
    let SimCapture {
        checkpoint: loaded_checkpoint,
        elements,
        settings,
        generation,
    } = loaded.capture;
    let source = generation.expect("generated").generator().expect("valid");
    let restored = Simulation::restore(elements, settings, Box::new(source), loaded_checkpoint)
        .expect("the loaded save restores");

    assert!(
        restored.checkpoint() == checkpoint,
        "the restored simulation differs"
    );
}

/// The shipped Elements as a hot reload might leave them: Oxygen denser, and a new Element
/// appended at the end.
fn reloaded_elements() -> ElementTable {
    let mut reloaded = ElementTable::new();
    for (_, element) in shipped_elements().iter().skip(1) {
        let mut element = element.clone();
        if element.name == "Oxygen" {
            element.density *= 2.0;
        }
        reloaded.add(element);
    }
    reloaded.add(
        Element::new("Sand")
            .density(1_600.0)
            .specific_heat(0.83)
            .conductivity(0.25)
            .phase(Phase::Solid),
    );
    reloaded
}

/// [`changed_save`] as a development build saves it after hot reloads: the simulation runs the
/// reloaded Elements under other settings, and the generator keeps the ones it started with.
fn hot_reloaded_save() -> WorldSave {
    let mut save = changed_save(SEED);
    let reloaded = reloaded_elements();
    save.capture.checkpoint.element_names = names(&reloaded);
    save.capture.elements = reloaded;
    save.capture.settings = SimSettings {
        conduction_multiplier: 250.0,
        gas_escape_fraction: 0.02,
        ..shipped_settings()
    };
    save
}

#[test]
fn a_development_save_restores_its_own_rules_whatever_the_files_say_now() {
    let dir = TempDir::new("dev-rules");
    let save = hot_reloaded_save();
    let id = dev_store(dir.path())
        .create("Tinkering", &save, at(5))
        .expect("created")
        .id;

    // A development store reads no asset files: the save's own rules come back.
    let loaded = dev_store(dir.path()).load(&id).expect("loaded").save;

    assert_same(&loaded, &save);
    assert_eq!(
        loaded.capture.generation,
        Some(Generation {
            seed: SEED,
            elements: shipped_elements(),
        })
    );
    assert_eq!(loaded.capture.elements, reloaded_elements());
    assert_eq!(loaded.capture.settings.conduction_multiplier, 250.0);
}

#[test]
fn a_release_build_loads_only_worlds_saved_under_the_rules_it_ships_and_leaves_others_as_they_are()
{
    let dir = TempDir::new("release-rules");
    let store = release_store(dir.path());
    let shipped = store
        .create("Shipped", &changed_save(SEED), at(1))
        .expect("created");
    assert_same(
        &store.load(&shipped.id).expect("loaded").save,
        &changed_save(SEED),
    );

    let other_generation = {
        let mut save = changed_save(SEED);
        let generation = save.capture.generation.as_mut().unwrap();
        generation.elements = reloaded_elements();
        save
    };
    let other_elements = {
        let mut save = hot_reloaded_save();
        save.capture.settings = shipped_settings();
        save
    };
    let other_settings = {
        let mut save = changed_save(SEED);
        save.capture.settings.conduction_multiplier = 999.0;
        save
    };
    for (save, expected) in [
        (other_generation, Incompatibility::GeneratorElements),
        (other_elements, Incompatibility::SimulationElements),
        (other_settings, Incompatibility::SimulationSettings),
    ] {
        let id = dev_store(dir.path())
            .create("Elsewhere", &save, at(2))
            .expect("created")
            .id;
        let path = store.path_of(&id);
        let before = file_state(&path);

        let refused = store.load(&id);

        assert!(
            matches!(&refused, Err(StoreError::Incompatible(why)) if *why == expected),
            "expected {expected:?}, got {refused:?}"
        );
        assert_eq!(file_state(&path), before, "loading changed the save");
        // A development build still loads it.
        assert_same(
            &dev_store(dir.path()).load(&id).expect("loaded").save,
            &save,
        );
    }
}

#[test]
fn unsupported_format_and_rule_revisions_are_refused_without_a_write() {
    let dir = TempDir::new("revisions");
    let store = dev_store(dir.path());
    let id = store
        .create("From the future", &changed_save(SEED), at(7))
        .expect("created")
        .id;
    let path = store.path_of(&id);
    let committed = fs::read(&path).expect("readable");

    let mut future_format = committed.clone();
    future_format[8..12].copy_from_slice(&99u32.to_le_bytes());
    let generation = worldgen::RULES_REVISION;
    let simulation = sim::RULES_REVISION;
    let future_generation = patch_header(
        &committed,
        &format!("generation_rules: {generation},"),
        &format!("generation_rules: {},", generation + 41),
    );
    let future_simulation = patch_header(
        &committed,
        &format!("simulation_rules: {simulation},"),
        &format!("simulation_rules: {},", simulation + 41),
    );
    let cases = [
        (
            future_format,
            Incompatibility::Format {
                saved: 99,
                supported: 1,
            },
        ),
        (
            future_generation,
            Incompatibility::GenerationRules {
                saved: generation + 41,
                supported: generation,
            },
        ),
        (
            future_simulation,
            Incompatibility::SimulationRules {
                saved: simulation + 41,
                supported: simulation,
            },
        ),
    ];
    for (bytes, expected) in cases {
        fs::write(&path, &bytes).expect("written");
        let before = file_state(&path);

        let loaded = store.load(&id);
        let entries = store.list().expect("listed");

        assert!(
            matches!(&loaded, Err(StoreError::Incompatible(why)) if *why == expected),
            "expected {expected:?}, got {loaded:?}"
        );
        let [entry] = entries.as_slice() else {
            panic!("one World is listed: {entries:?}");
        };
        match (&entry.summary, &expected) {
            // A format this build can't read has no header it can read either.
            (Err(StoreError::Incompatible(why)), Incompatibility::Format { .. }) => {
                assert_eq!(*why, expected);
            }
            // Other revisions show in a listing the World's name and seed.
            (Ok(summary), _) => {
                assert_eq!(summary.name, "From the future");
                assert_eq!(summary.compatibility, Err(expected.clone()));
            }
            (other, _) => panic!("unexpected listing {other:?} for {expected:?}"),
        }
        // A save from a newer format can't be rewritten safely, so it isn't renamed.
        if let Incompatibility::Format { .. } = expected {
            assert!(matches!(
                store.rename(&id, "Renamed"),
                Err(StoreError::Incompatible(_))
            ));
        }
        assert_eq!(file_state(&path), before, "reading changed the save");
    }
    // A save the build can't load can still be deleted.
    store.delete(&id).expect("deleted");
    assert!(store.list().expect("listed").is_empty());
}

// Seam 3: failed, interrupted and stale writes, and damage.

#[test]
fn a_save_that_fails_after_staging_leaves_the_earlier_one_loadable() {
    let dir = TempDir::new("failed-save");
    let fresh = fresh_save(SEED);
    let id = dev_store(dir.path())
        .create("Steady", &fresh, at(1_000))
        .expect("created")
        .id;
    let failing =
        dev_store(dir.path()).before_publish(|_| Err(io::Error::other("the disk is full")));

    let failed = failing.save(failing.ticket(&id), &changed_save(SEED), at(2_000));

    assert!(matches!(failed, Err(StoreError::Io(_))), "{failed:?}");
    drop(failing);
    let reopened = dev_store(dir.path());
    let loaded = reopened.load(&id).expect("loaded");
    assert_same(&loaded.save, &fresh);
    assert_eq!(loaded.summary.last_played, at(1_000));
    // The staged file was cleared away, so only the World's own file is left.
    assert_eq!(files_in(dir.path()), [format!("{id}.world")]);
}

#[test]
fn while_a_replacement_is_cut_short_the_committed_save_is_the_one_that_loads() {
    let dir = TempDir::new("interrupted");
    let fresh = fresh_save(SEED);
    let id = dev_store(dir.path())
        .create("Steady", &fresh, at(1_000))
        .expect("created")
        .id;
    let (staged, staging) = mpsc::channel();
    let (resume, resumed) = mpsc::channel::<io::Result<()>>();
    let resumed = Mutex::new(resumed);
    let writer = dev_store(dir.path()).before_publish(move |_| {
        staged.send(()).expect("the test is waiting");
        resumed.lock().unwrap().recv().expect("the test resumes")
    });
    let writing =
        thread::spawn(move || writer.save(writer.ticket(&id), &changed_save(SEED), at(2_000)));
    staging.recv_timeout(TIMEOUT).expect("the save is staged");

    // The new save is staged beside the committed one but not published: storage opened now,
    // as after a crash, finds the committed save.
    let reopened = dev_store(dir.path());
    let entries = reopened.list().expect("listed");
    let loaded = reopened.load(&id).expect("loaded");

    assert_eq!(entries.len(), 1);
    let summary = entries[0].summary.as_ref().expect("readable");
    assert_eq!(summary.last_played, at(1_000));
    assert_same(&loaded.save, &fresh);

    resume
        .send(Err(io::Error::new(io::ErrorKind::Interrupted, "power cut")))
        .expect("the writer waits");
    let written = writing.join().expect("the writer finishes");
    assert!(matches!(written, Err(StoreError::Io(_))), "{written:?}");
    assert_same(&reopened.load(&id).expect("loaded").save, &fresh);
}

#[test]
fn a_world_whose_first_write_fails_is_never_created() {
    let dir = TempDir::new("failed-create");
    let failing =
        dev_store(dir.path()).before_publish(|_| Err(io::Error::other("the disk is full")));

    let created = failing.create("Stillborn", &fresh_save(SEED), at(1));

    assert!(matches!(created, Err(StoreError::Io(_))), "{created:?}");
    assert!(dev_store(dir.path()).list().expect("listed").is_empty());
    assert!(
        files_in(dir.path()).is_empty(),
        "{:?}",
        files_in(dir.path())
    );
}

#[test]
fn a_save_that_finishes_after_a_newer_one_is_dropped() {
    let dir = TempDir::new("stale");
    let store = dev_store(dir.path());
    let id = store
        .create("Racing", &fresh_save(SEED), at(1))
        .expect("created")
        .id;
    let newer_state = changed_save(SEED);
    let mut older_state = changed_save(SEED);
    older_state.capture.checkpoint.tick_count -= 100;

    // Sampled in this order, but the older save is slower to encode and finishes last.
    let older = store.ticket(&id);
    let newer = store.ticket(&id);
    store.save(newer, &newer_state, at(3)).expect("saved");
    let stale = store.save(older, &older_state, at(2));

    assert!(matches!(stale, Err(StoreError::Superseded)), "{stale:?}");
    let loaded = store.load(&id).expect("loaded");
    assert_eq!(loaded.summary.last_played, at(3));
    assert_same(&loaded.save, &newer_state);

    // Saves that finish in the order they were sampled all commit.
    let first = store.ticket(&id);
    let second = store.ticket(&id);
    store.save(first, &older_state, at(4)).expect("saved");
    store.save(second, &fresh_save(SEED), at(5)).expect("saved");
    assert_same(&store.load(&id).expect("loaded").save, &fresh_save(SEED));
}

#[test]
fn writes_to_one_world_take_turns() {
    let dir = TempDir::new("turns");
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&events);
    let store = dev_store(dir.path()).before_publish(move |_| {
        recorded.lock().unwrap().push("publishing");
        // Long enough for another write to barge in, if it could.
        thread::sleep(Duration::from_millis(20));
        recorded.lock().unwrap().push("published");
        Ok(())
    });
    let id = store
        .create("Busy", &fresh_save(SEED), at(1))
        .expect("created")
        .id;

    let writers: Vec<_> = (0..4)
        .map(|i| {
            let (store, id) = (store.clone(), id);
            thread::spawn(move || match i {
                0 => store.rename(&id, "Renamed").map(|_| ()),
                _ => store
                    .save(store.ticket(&id), &changed_save(SEED), at(10 + i))
                    .map(|_| ()),
            })
        })
        .collect();
    for writer in writers {
        // A save whose ticket a newer one beat is dropped, which is fine here.
        match writer.join().expect("the writer finishes") {
            Ok(()) | Err(StoreError::Superseded) => {}
            Err(error) => panic!("{error}"),
        }
    }

    let events = events.lock().unwrap();
    assert!(events.len() >= 4, "{events:?}");
    for pair in events.chunks(2) {
        assert_eq!(pair, ["publishing", "published"], "{events:?}");
    }
    let loaded = store.load(&id).expect("loaded");
    assert_eq!(loaded.summary.name, "Renamed");
    assert_same(&loaded.save, &changed_save(SEED));
}

#[test]
fn a_save_after_its_world_is_deleted_fails_and_never_brings_it_back() {
    let dir = TempDir::new("deleted");
    let store = dev_store(dir.path());
    let id = store
        .create("Doomed", &fresh_save(SEED), at(1))
        .expect("created")
        .id;
    let ticket = store.ticket(&id);

    store.delete(&id).expect("deleted");
    let saved = store.save(ticket, &changed_save(SEED), at(2));

    assert!(matches!(saved, Err(StoreError::NotFound(_))), "{saved:?}");
    assert!(store.list().expect("listed").is_empty());
    assert!(files_in(dir.path()).is_empty());
}

#[test]
fn a_save_must_come_from_the_worlds_own_seed_and_generator() {
    let dir = TempDir::new("seed");
    let store = dev_store(dir.path());
    let id = store
        .create("Seeded", &fresh_save(SEED), at(1))
        .expect("created")
        .id;
    let mut not_generated = fresh_save(SEED);
    not_generated.capture.generation = None;

    let other_seed = store.save(store.ticket(&id), &fresh_save(SEED + 1), at(2));
    let saved = store.save(store.ticket(&id), &not_generated, at(3));
    let created = store.create("Nowhere", &not_generated, at(4));

    assert!(
        matches!(
            other_seed,
            Err(StoreError::SeedMismatch {
                world: SEED,
                save: 2027
            })
        ),
        "{other_seed:?}"
    );
    assert!(matches!(saved, Err(StoreError::NotGenerated)), "{saved:?}");
    assert!(
        matches!(created, Err(StoreError::NotGenerated)),
        "{created:?}"
    );
    assert_eq!(store.load(&id).expect("loaded").summary.last_played, at(1));
}

#[test]
fn a_damaged_world_reports_its_own_error_and_the_others_stay_usable() {
    let dir = TempDir::new("damaged");
    let store = dev_store(dir.path());
    let fresh = fresh_save(SEED);
    let create = |name, ms| store.create(name, &fresh, at(ms)).expect("created").id;
    let healthy = create("Healthy", 3);
    let flipped = create("Flipped", 4);
    let garbled = create("Garbled", 2);
    let truncated = create("Truncated", 1);
    let path = |id: &WorldId| store.path_of(id);

    // A bit flipped in the compressed Cells at the end of the file, a file overwritten with
    // something else entirely, and a file cut short.
    let mut bytes = fs::read(path(&flipped)).expect("readable");
    let last = bytes.len() - 1;
    bytes[last] ^= 0x10;
    fs::write(path(&flipped), bytes).expect("written");
    fs::write(path(&garbled), "not a World save at all").expect("written");
    let bytes = fs::read(path(&truncated)).expect("readable");
    fs::write(path(&truncated), &bytes[..bytes.len() - 100]).expect("written");
    let before: Vec<_> = [&flipped, &garbled, &truncated]
        .map(|id| file_state(&path(id)))
        .into();

    let entries = store.list().expect("listed");

    // The catalog reads headers only, so it can't see damage to the Cells. The damaged
    // listings come last, by id.
    let mut damaged = [garbled, truncated];
    damaged.sort();
    let listed: Vec<(WorldId, bool)> = entries
        .iter()
        .map(|entry| (entry.id, entry.summary.is_ok()))
        .collect();
    assert_eq!(
        listed,
        [
            (flipped, true),
            (healthy, true),
            (damaged[0], false),
            (damaged[1], false),
        ]
    );
    for entry in &entries[2..] {
        assert!(
            matches!(entry.summary, Err(StoreError::Corrupt(_))),
            "{entry:?}"
        );
    }
    for id in [&flipped, &garbled, &truncated] {
        let loaded = store.load(id);
        assert!(matches!(loaded, Err(StoreError::Corrupt(_))), "{loaded:?}");
    }
    let after: Vec<_> = [&flipped, &garbled, &truncated]
        .map(|id| file_state(&path(id)))
        .into();
    assert!(after == before, "reading changed a damaged save");
    assert_same(&store.load(&healthy).expect("loaded").save, &fresh);

    // A damaged World can still be deleted, and the healthy one carries on.
    for id in [&flipped, &garbled, &truncated] {
        store.delete(id).expect("deleted");
    }
    assert_eq!(listed_ids(&store), [healthy]);
    store
        .save(store.ticket(&healthy), &changed_save(SEED), at(9))
        .expect("saved");
}

// Seam 4: the catalog's order.

#[test]
fn the_catalog_lists_the_most_recently_played_first_and_breaks_ties_by_name_then_id() {
    let dir = TempDir::new("order");
    let store = dev_store(dir.path());
    let fresh = fresh_save(SEED);
    let create = |name, ms| store.create(name, &fresh, at(ms)).expect("created").id;
    let bravo = create("Bravo", 2_000);
    let oldest = create("Alpha", 1_000);
    let charlie = create("Charlie", 3_000);
    let mut tied = [create("Alpha", 2_000), create("Alpha", 2_000)];
    tied.sort();
    let order = |store: &WorldStore| -> Vec<(WorldId, String, SystemTime)> {
        store
            .list()
            .expect("listed")
            .into_iter()
            .map(|entry| {
                let summary = entry.summary.expect("readable");
                (entry.id, summary.name, summary.last_played)
            })
            .collect()
    };

    assert_eq!(
        order(&store),
        [
            (charlie, "Charlie".into(), at(3_000)),
            (tied[0], "Alpha".into(), at(2_000)),
            (tied[1], "Alpha".into(), at(2_000)),
            (bravo, "Bravo".into(), at(2_000)),
            (oldest, "Alpha".into(), at(1_000)),
        ]
    );

    // Saving a World brings it to the top; renaming one leaves it where it was.
    store
        .save(store.ticket(&oldest), &fresh, at(4_000))
        .expect("saved");
    store.rename(&charlie, "Aardvark").expect("renamed");

    let expected = [
        (oldest, "Alpha".into(), at(4_000)),
        (charlie, "Aardvark".into(), at(3_000)),
        (tied[0], "Alpha".into(), at(2_000)),
        (tied[1], "Alpha".into(), at(2_000)),
        (bravo, "Bravo".into(), at(2_000)),
    ];
    assert_eq!(order(&store), expected);
    // The order survives reopening the store.
    assert_eq!(order(&dev_store(dir.path())), expected);
}

#[test]
fn opening_a_store_creates_its_folder_and_lists_only_world_saves() {
    let dir = TempDir::new("open");
    let saves = dir.path().join("deeply").join("nested").join("saves");
    let store = dev_store(&saves);
    assert!(saves.is_dir());
    assert_eq!(store.dir(), saves.as_path());

    // Files that aren't World saves are left alone and not listed.
    fs::write(saves.join("notes.txt"), "hello").expect("written");
    fs::write(saves.join("0123456789abcdef.txt"), "hello").expect("written");
    fs::write(saves.join("not-an-id.world"), "hello").expect("written");
    fs::create_dir(saves.join("fedcba9876543210.world")).expect("created");
    let id = store
        .create("Real", &fresh_save(SEED), at(1))
        .expect("created")
        .id;

    let ids = listed_ids(&store);
    assert_eq!(ids, [id]);
}
