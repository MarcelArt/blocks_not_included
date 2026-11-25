//! The layout of a World's save file.
//!
//! ```text
//! offset   size  what
//! 0        8     magic: `BNIWORLD`
//! 8        4     the format revision, a little-endian u32: `FORMAT`
//! 12       4     the header's length in bytes, a little-endian u32: n
//! 16       n     the header: a `Header`, as UTF-8 RON
//! 16 + n   m     the body: zlib-compressed; the header gives its length m and its CRC-32
//! ```
//!
//! The header holds what the catalog lists, so a listing reads the first few hundred bytes of
//! each file and never its Cells. The body holds, before compression, in little-endian binary:
//!
//! 1. the rules, each as RON text: the Elements the world generator was built with and the
//!    Elements the simulation ran with, both as Elements files, then the simulation settings;
//! 2. the player: feet, velocity, yaw and pitch as f32, then movement mode and grounded as u8;
//! 3. the checkpoint: Element names; focus; the Active Region's placement; Tick count; Mass lost
//!    to space; then each Chunk with its Settled flag and contents. A Chunk's Cells are stored a
//!    field at a time: every Element index, then every Mass, then every temperature's bits,
//!    which compresses far better than Cell by Cell.
//!
//! Text is a u32 byte length followed by UTF-8. Every float is stored by its bits, so it comes
//! back exactly, NaN payloads and −0 included.

use std::io::{self, Read, Write};

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use serde::{Deserialize, Serialize};
use sim::{
    Cell, CellPos, Checkpoint, ChunkCheckpoint, ChunkContents, ChunkPos, ElementTable, SimSettings,
};

use super::{GameMode, PlayerState, StoreError, WorldSave};
use crate::player::movement::{Body, MoveMode};
use crate::sim_bridge::{Generation, SimCapture};

/// The first bytes of every save file.
const MAGIC: [u8; 8] = *b"BNIWORLD";

/// The revision of this layout. Bump it with any change to the layout, including the header's
/// fields and the body's encoding. Builds read only their own revision: migrating saves is
/// deferred.
pub(super) const FORMAT: u32 = 1;

/// The magic, the format revision and the header's length.
const PREFIX_LEN: usize = 16;

/// The longest header read. Real ones are a few hundred bytes; this only stops a damaged length
/// from asking for gigabytes.
const MAX_HEADER_LEN: u32 = 64 * 1024;

/// What a save file says about its World before the body: everything the catalog lists, and
/// what's needed to check the body.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Header {
    pub name: String,
    pub seed: u64,
    /// When the World was last played, in milliseconds since the Unix epoch.
    pub last_played_ms: u64,
    pub mode: GameMode,
    /// The [`worldgen::RULES_REVISION`] the World's terrain was generated under.
    pub generation_rules: u32,
    /// The [`sim::RULES_REVISION`] the checkpoint was captured under.
    pub simulation_rules: u32,
    /// The body's length in bytes, as stored.
    pub body_len: u64,
    /// The CRC-32 of the body, as stored.
    pub body_crc32: u32,
}

impl Header {
    /// The header of a save of `body` under this build's rules.
    pub fn new(name: String, seed: u64, last_played_ms: u64, body: &[u8]) -> Self {
        Self {
            name,
            seed,
            last_played_ms,
            mode: GameMode::Sandbox,
            generation_rules: worldgen::RULES_REVISION,
            simulation_rules: sim::RULES_REVISION,
            body_len: body.len() as u64,
            body_crc32: crc32fast::hash(body),
        }
    }
}

/// Damage found while reading a save, in words.
fn corrupt(what: impl Into<String>) -> StoreError {
    StoreError::Corrupt(what.into())
}

/// The whole save file: the prefix, `header` and `body`.
pub(super) fn assemble(header: &Header, body: &[u8]) -> Vec<u8> {
    let header = ron::ser::to_string_pretty(header, ron::ser::PrettyConfig::default())
        .expect("a header can be written as RON");
    let header_len = u32::try_from(header.len()).expect("a header is short");
    let mut file = Vec::with_capacity(PREFIX_LEN + header.len() + body.len());
    file.extend_from_slice(&MAGIC);
    file.extend_from_slice(&FORMAT.to_le_bytes());
    file.extend_from_slice(&header_len.to_le_bytes());
    file.extend_from_slice(header.as_bytes());
    file.extend_from_slice(body);
    file
}

/// Reads a save file's prefix and header from `file`, which is `file_len` bytes long, and checks
/// that the file's length is what the header says. Reads nothing of the body.
pub(super) fn read_header(file: &mut impl Read, file_len: u64) -> Result<Header, StoreError> {
    let mut prefix = [0; PREFIX_LEN];
    file.read_exact(&mut prefix)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => corrupt("the file is too short to be a World save"),
            _ => StoreError::Io(error),
        })?;
    if prefix[..8] != MAGIC {
        return Err(corrupt("the file isn't a World save"));
    }
    let format = u32::from_le_bytes(prefix[8..12].try_into().expect("4 bytes"));
    if format != FORMAT {
        return Err(StoreError::Incompatible(super::Incompatibility::Format {
            saved: format,
            supported: FORMAT,
        }));
    }
    let header_len = u32::from_le_bytes(prefix[12..16].try_into().expect("4 bytes"));
    if header_len > MAX_HEADER_LEN {
        return Err(corrupt(format!("the header claims {header_len} bytes")));
    }
    let mut text = vec![0; header_len as usize];
    file.read_exact(&mut text)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => corrupt("the file ends inside its header"),
            _ => StoreError::Io(error),
        })?;
    let text = std::str::from_utf8(&text).map_err(|_| corrupt("the header isn't text"))?;
    let header: Header = ron::from_str(text)
        .map_err(|error| corrupt(format!("the header is unreadable: {error}")))?;
    let expected_len = (PREFIX_LEN as u64 + u64::from(header_len)).saturating_add(header.body_len);
    if file_len < expected_len {
        return Err(corrupt(format!(
            "the file is cut short: {file_len} bytes of {expected_len}"
        )));
    }
    if file_len > expected_len {
        return Err(corrupt(format!(
            "the file has {} bytes too many",
            file_len - expected_len
        )));
    }
    Ok(header)
}

/// A whole save file's header and its body, as stored.
pub(super) fn split(bytes: &[u8]) -> Result<(Header, &[u8]), StoreError> {
    let mut reader = bytes;
    let header = read_header(&mut reader, bytes.len() as u64)?;
    // `read_header` checked that exactly the body is left.
    Ok((header, reader))
}

/// The body of a save of `save`, compressed. `save` must hold a [`Generation`].
pub(super) fn encode_body(save: &WorldSave, generation: &Generation) -> Vec<u8> {
    let mut raw = Writer::default();
    let capture = &save.capture;
    raw.text(&generation.elements.to_ron());
    raw.text(&capture.elements.to_ron());
    raw.text(&ron::to_string(&capture.settings).expect("settings can be written as RON"));
    raw.player(&save.player);
    raw.checkpoint(&capture.checkpoint);

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder
        .write_all(&raw.0)
        .and_then(|()| encoder.finish())
        .expect("compressing into memory can't fail")
}

/// The rules a save's body holds, before its player and checkpoint.
pub(super) struct SavedRules {
    pub generator_elements: ElementTable,
    pub elements: ElementTable,
    pub settings: SimSettings,
}

/// Reads a save's body, as stored, and checks it against `header`. `check_rules` sees the rules
/// before anything else is decoded, and can refuse them.
pub(super) fn decode_body(
    header: &Header,
    body: &[u8],
    check_rules: impl FnOnce(&SavedRules) -> Result<(), StoreError>,
) -> Result<WorldSave, StoreError> {
    if crc32fast::hash(body) != header.body_crc32 {
        return Err(corrupt("its contents don't match their checksum"));
    }
    let mut raw = Vec::new();
    ZlibDecoder::new(body)
        .read_to_end(&mut raw)
        .map_err(|error| corrupt(format!("its contents can't be decompressed: {error}")))?;
    let mut reader = Reader(&raw);

    let rules = reader.rules()?;
    check_rules(&rules)?;
    let player = reader.player()?;
    let checkpoint = reader.checkpoint(&rules.elements)?;
    if !reader.0.is_empty() {
        return Err(corrupt("its contents run on past the checkpoint"));
    }
    let SavedRules {
        generator_elements,
        elements,
        settings,
    } = rules;
    Ok(WorldSave {
        capture: SimCapture {
            checkpoint,
            elements,
            settings,
            generation: Some(Generation {
                seed: header.seed,
                elements: generator_elements,
            }),
        },
        player,
    })
}

/// Writes a body's fields.
#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn f32(&mut self, value: f32) {
        self.u32(value.to_bits());
    }

    fn len(&mut self, len: usize) {
        self.u32(u32::try_from(len).expect("fewer than 2³² items"));
    }

    fn text(&mut self, text: &str) {
        self.len(text.len());
        self.0.extend_from_slice(text.as_bytes());
    }

    fn player(&mut self, player: &PlayerState) {
        let PlayerState { body, yaw, pitch } = *player;
        let Body {
            feet,
            velocity,
            mode,
            grounded,
        } = body;
        for value in feet.to_array().into_iter().chain(velocity.to_array()) {
            self.f32(value);
        }
        self.f32(yaw);
        self.f32(pitch);
        self.u8(match mode {
            MoveMode::Walk => 0,
            MoveMode::Fly => 1,
        });
        self.u8(u8::from(grounded));
    }

    fn checkpoint(&mut self, checkpoint: &Checkpoint) {
        let Checkpoint {
            element_names,
            focus,
            placement_min,
            placement_chunks,
            tick_count,
            mass_lost_to_space_grams,
            chunks,
        } = checkpoint;
        self.len(element_names.len());
        for name in element_names {
            self.text(name);
        }
        for value in [focus.x, focus.y, focus.z] {
            self.i32(value);
        }
        self.chunk_pos(*placement_min);
        for value in placement_chunks {
            self.u32(*value);
        }
        self.u64(*tick_count);
        self.u64(*mass_lost_to_space_grams);
        self.len(chunks.len());
        for (&pos, chunk) in chunks {
            self.chunk_pos(pos);
            self.u8(u8::from(chunk.settled));
            match &chunk.contents {
                ChunkContents::Generated => self.u8(0),
                ChunkContents::Vacuum => self.u8(1),
                ChunkContents::Cells(cells) => {
                    self.u8(2);
                    self.cells(cells);
                }
            }
        }
    }

    fn chunk_pos(&mut self, pos: ChunkPos) {
        for value in [pos.x, pos.y, pos.z] {
            self.i32(value);
        }
    }

    /// The Cells a field at a time, so each field's runs of equal values compress.
    fn cells(&mut self, cells: &[Cell]) {
        self.len(cells.len());
        self.0.reserve(cells.len() * CELL_BYTES);
        for cell in cells {
            self.0
                .extend_from_slice(&cell.element.index().to_le_bytes());
        }
        for cell in cells {
            self.u32(cell.mass_grams);
        }
        for cell in cells {
            self.f32(cell.temperature_k);
        }
    }
}

/// The bytes one Cell takes: a u16 Element index, a u32 Mass and an f32 temperature.
const CELL_BYTES: usize = 2 + 4 + 4;

/// Reads a body's fields, failing with [`StoreError::Corrupt`] where they run out or make no
/// sense.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], StoreError> {
        if self.0.len() < len {
            return Err(corrupt("its contents end too soon"));
        }
        let (taken, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(taken)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], StoreError> {
        Ok(self.take(N)?.try_into().expect("N bytes"))
    }

    fn u8(&mut self) -> Result<u8, StoreError> {
        Ok(self.array::<1>()?[0])
    }

    fn u32(&mut self) -> Result<u32, StoreError> {
        self.array().map(u32::from_le_bytes)
    }

    fn i32(&mut self) -> Result<i32, StoreError> {
        self.array().map(i32::from_le_bytes)
    }

    fn u64(&mut self) -> Result<u64, StoreError> {
        self.array().map(u64::from_le_bytes)
    }

    fn f32(&mut self) -> Result<f32, StoreError> {
        self.u32().map(f32::from_bits)
    }

    fn flag(&mut self, what: &str) -> Result<bool, StoreError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(corrupt(format!("{what} is {other}, neither 0 nor 1"))),
        }
    }

    /// A count of items at least `item_bytes` long each, checked against what's left, so a
    /// damaged count can't ask for more memory than the body could fill.
    fn len(&mut self, item_bytes: usize) -> Result<usize, StoreError> {
        let len = self.u32()? as usize;
        if len.saturating_mul(item_bytes) > self.0.len() {
            return Err(corrupt("its contents end too soon"));
        }
        Ok(len)
    }

    fn text(&mut self) -> Result<&'a str, StoreError> {
        let len = self.len(1)?;
        std::str::from_utf8(self.take(len)?).map_err(|_| corrupt("its text isn't UTF-8"))
    }

    fn rules(&mut self) -> Result<SavedRules, StoreError> {
        let generator_elements = ElementTable::from_ron(self.text()?).map_err(|error| {
            corrupt(format!(
                "the Elements its terrain comes from are invalid: {error}"
            ))
        })?;
        let elements = ElementTable::from_ron(self.text()?).map_err(|error| {
            corrupt(format!(
                "the Elements it was simulated with are invalid: {error}"
            ))
        })?;
        let settings: SimSettings = ron::from_str(self.text()?)
            .map_err(|error| corrupt(format!("its simulation settings are unreadable: {error}")))?;
        settings
            .validate()
            .map_err(|error| corrupt(format!("its simulation settings are invalid: {error}")))?;
        Ok(SavedRules {
            generator_elements,
            elements,
            settings,
        })
    }

    fn player(&mut self) -> Result<PlayerState, StoreError> {
        let mut vector = || -> Result<_, StoreError> {
            Ok(bevy::math::Vec3::new(self.f32()?, self.f32()?, self.f32()?))
        };
        let feet = vector()?;
        let velocity = vector()?;
        let yaw = self.f32()?;
        let pitch = self.f32()?;
        let mode = match self.u8()? {
            0 => MoveMode::Walk,
            1 => MoveMode::Fly,
            other => return Err(corrupt(format!("the player's movement mode is {other}"))),
        };
        let grounded = self.flag("whether the player is grounded")?;
        Ok(PlayerState {
            body: Body {
                feet,
                velocity,
                mode,
                grounded,
            },
            yaw,
            pitch,
        })
    }

    /// The checkpoint, whose Cells hold Elements of `elements`.
    fn checkpoint(&mut self, elements: &ElementTable) -> Result<Checkpoint, StoreError> {
        let names = self.len(4)?;
        let element_names = (0..names)
            .map(|_| self.text().map(str::to_owned))
            .collect::<Result<_, _>>()?;
        let focus = CellPos::new(self.i32()?, self.i32()?, self.i32()?);
        let placement_min = self.chunk_pos()?;
        let placement_chunks = [self.u32()?, self.u32()?, self.u32()?];
        let tick_count = self.u64()?;
        let mass_lost_to_space_grams = self.u64()?;
        // Position, Settled flag and kind of contents: 14 bytes.
        let count = self.len(14)?;
        let mut chunks = std::collections::BTreeMap::new();
        for _ in 0..count {
            let pos = self.chunk_pos()?;
            let settled = self.flag("whether a Chunk is Settled")?;
            let contents = match self.u8()? {
                0 => ChunkContents::Generated,
                1 => ChunkContents::Vacuum,
                2 => ChunkContents::Cells(self.cells(elements)?),
                other => return Err(corrupt(format!("a Chunk's contents are of kind {other}"))),
            };
            if chunks
                .insert(pos, ChunkCheckpoint { contents, settled })
                .is_some()
            {
                return Err(corrupt("a Chunk is listed twice"));
            }
        }
        Ok(Checkpoint {
            element_names,
            focus,
            placement_min,
            placement_chunks,
            tick_count,
            mass_lost_to_space_grams,
            chunks,
        })
    }

    fn chunk_pos(&mut self) -> Result<ChunkPos, StoreError> {
        Ok(ChunkPos::new(self.i32()?, self.i32()?, self.i32()?))
    }

    fn cells(&mut self, elements: &ElementTable) -> Result<Vec<Cell>, StoreError> {
        let len = self.len(CELL_BYTES)?;
        let indices = self.take(len * 2)?;
        let masses = self.take(len * 4)?;
        let temperatures = self.take(len * 4)?;
        indices
            .chunks_exact(2)
            .zip(masses.chunks_exact(4))
            .zip(temperatures.chunks_exact(4))
            .map(|((index, mass), temperature)| {
                let index = u16::from_le_bytes(index.try_into().expect("2 bytes"));
                let element = elements.id_at(index).ok_or_else(|| {
                    corrupt(format!(
                        "a Cell holds Element #{index}, which isn't in its table"
                    ))
                })?;
                Ok(Cell {
                    element,
                    mass_grams: u32::from_le_bytes(mass.try_into().expect("4 bytes")),
                    temperature_k: f32::from_bits(u32::from_le_bytes(
                        temperature.try_into().expect("4 bytes"),
                    )),
                })
            })
            .collect()
    }
}
