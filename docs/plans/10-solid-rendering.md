# Step 10: Rendering solid Cells

> Read `docs/plans/CONVENTIONS.md` and `docs/plans/BEVY-0.20-NOTES.md` first.

**Prerequisites:** step 09 is done (`WorldView`, plus the per-frame record of changed Chunks).

## Goal

Every loaded Chunk's solid Cells are drawn as flat-coloured cubes, built with `binary-greedy-meshing`, and are remeshed off the main thread whenever they change.

## Design

- **Module.** `src/render/`, with a `ChunkMeshPlugin`.
- **Mesher input.**
  - Build the padded `34³` `u16` array that `Mesher::<32>` expects: the Chunk's own Cells plus one layer of border Cells from each face neighbour.
  - Value 0 means "not solid"; any other value is `ElementId::index() + 1`.
  - A missing neighbour Chunk counts as not solid, so its faces are drawn.
  - Only Cells in the solid Phase are meshed here. Liquids and gases come in step 13.
- **Mesh.**
  - Convert the quads into positions, normals and vertex colours, taking each Element's colour from the `ElementTable` resource.
  - All Chunks share one `StandardMaterial` with a white base colour, so the vertex colours show.
  - Each Chunk entity has `Mesh3d`, `MeshMaterial3d` and a `Transform` at `ChunkPos::origin()`.
- **Remeshing.**
  - Remesh a Chunk when it, or any of its face neighbours, changed.
  - Meshing runs on `AsyncComputeTaskPool`, with a budget on how many tasks start each frame.
  - Despawn a Chunk's entity when the Chunk leaves `WorldView`.
- **Scene.** A directional sun light, ambient light and a sky clear colour. Add a temporary free-fly camera so the world can be seen; step 11 replaces it.
- **Ambient occlusion.** Optional. If the quad data makes per-vertex AO easy, add it; otherwise note it as deferred in your report.

### Decisions made during implementation

- **Modules.**
  - `src/render/mesher.rs` holds the pure functions: `SolidColors`, `ChunkNeighbourhood::gather`, `mesh_solids` and `ChunkGeometry`.
  - `src/render/mod.rs` holds `ChunkMeshPlugin`, plus a `SolidMesh(ChunkPos)` component on each Chunk entity.
  - `src/render/scene.rs` holds `ScenePlugin`: the sky, the sun and the camera. It is separate so step 11 can replace the camera without touching the meshing.
- **Mesher.**
  - `binary-greedy-meshing` 0.6.4, using `fast_mesh` with a solid mask built while padding; no value is transparent.
  - The crate's quads and winding are used as they are: the tests check that every triangle winds counter-clockwise, seen from outside.
  - A Chunk with no solid Cell returns no geometry without padding. That is about half of render distance (554 of 1,183 Chunks).
- **Colours.** `SolidColors` is built from the `Elements` resource. When that resource changes, it is rebuilt and every Chunk is remeshed, so step 15 only needs to replace the resource.
- **Remeshing.**
  - A Chunk goes stale when it changes, or when a face neighbour changes or leaves `WorldView`. A neighbour leaving counts, so the edge of render distance gets closed with faces.
  - Up to 16 stale Chunks start meshing each frame, nearest the focus first, on `AsyncComputeTaskPool`.
  - Each Chunk has at most one task: starting a newer one drops, and so cancels, the older.
  - A result with no faces despawns the Chunk's entity. A Chunk leaving `WorldView` loses its task and its entity.
- **Mesh.** Meshes use `RenderAssetUsages::RENDER_WORLD`; each remesh adds a new mesh asset, and the old one is freed.
- **Lighting.** Bevy's default exposure (EV100 9.7) shows a lit face at about its colour × lux × cos θ / π × 0.001. So the values are:
  - a 2,500 lux sun, travelling along (0.5, −1, 0.3), with shadows out to render distance;
  - ambient light of brightness 450 in a pale sky blue.

  A sunlit top shows at about its Element's colour, the two lit sides at about 0.7 and 0.55 of it, and shade at about 0.35. The first try, a 10,000 lux sun, washed Dirt out to beige.
- **Camera.** This is Bevy's own `FreeCamera`, which needs Bevy's `free_camera` feature in `Cargo.toml`. Step 11 should remove the feature along with `FreeCameraPlugin`.
  - It starts 40 m back on −x and −z and 30 m up from the focus, looking at it.
  - Controls: hold the right mouse button (or press M) to look; WASD moves; Q and E go down and up; Shift runs; the scroll wheel changes speed.
- **Ambient occlusion: deferred.**
  - Greedy quads span many Cells, so per-vertex AO would need merging to stop where AO values differ, which means changing the mesher itself.
  - The crate doesn't compute AO: its `Quad` layout reserves two AO bits that `Quad::pack` never sets.
- **Frame rate.** Measured on the GTX 1650 with `prime-run`. Bevy's default `WinitSettings::game()` caps an unfocused window at 60 Hz, so the measurement used `WinitSettings::continuous()`.
  - Render distance filled in about 6 s.
  - From 2 s to 6 s, the game held 141–145 frames per second on a 144 Hz display, with the worst frame at 11 ms, while meshing kept pace with loading.
  - Only the first 1.5 s had slower frames, of 20–90 ms, before most of the world is drawn.

### Revised later: lighting in caves

Caves were hard to read. Out of the sun, every face got the same flat ambient light, so a floor, the wall beside it and a drop beyond it all showed in one shade of their Element's colour. These changes replaced the lighting decision and the deferred AO above:

- **Ambient occlusion.** Each corner of a solid face is darkened by the solid Cells that meet there, in the layer the face looks out into: to 0.76, 0.55 or 0.35 of its colour for one, two or three of them. Two Cells across the corner from each other count as three. Liquids and gases don't darken.
  - Every Cell in front of a greedy quad is open, so only the corners on its border can be darkened. A quad with any is split again, greedily, into runs whose corners match. Interpolating across each run then gives every Cell corner its exact shade.
  - Each quad is split into triangles along its less occluded diagonal, so a dark corner darkens only its own triangle.
  - The shading reads the edge and corner neighbour Chunks too. `ChunkNeighbourhood` gathers all 26 neighbours, and a Chunk goes stale when any of them changes or leaves.
  - Cost, at the dev profile's opt-level 1, on the 75 Chunks around spawn: solid meshing went from about 0.8 to 1.5 ms a Chunk, and solid meshes have 1.9 times as many vertices.
- **Sky light by which way a face points.** An `EnvironmentMapLight` on the camera, a 1×1 cubemap, replaces the flat ambient light (`GlobalAmbientLight::NONE`). It has intensity 550 on faces pointing up, 0.72 of that on ±x, 0.56 on ±z and 0.4 on faces pointing down. Sunlit faces look much as before. In shade, floors, the two kinds of wall and ceilings each have a shade of their own.
- **Dark where the sky doesn't reach.** With the sky light everywhere, a pocket sealed in underground was as bright as shade outside.
  - Every 0.1 s, 13 rays go up from the eyes: one straight up, and six each at 60° and 35° above the horizon. The share that meet no solid Cell within 48 m is the share of the sky seen.
  - A share of 0.4 or more gives full daylight, so a cliff or an overhang doesn't dim anything. Below that, daylight eases (smoothstep) to none.
  - The camera's sky light goes from 550 in full daylight to 55 with none, following daylight at a rate of 2 a second, like eyes adjusting. Sealed in, faces show at about 0.05 of their colour on floors, down to 0.02 on ceilings: dark, but every Cell's shape still shows by the shade of each face and its occlusion.
  - Every cave near spawn, even 6 m down, counts as sealed in. The dimming is the camera's, so a cave seen from outside is lit as before, and the sunlit world seen from inside a cave dims along with it.
- **A lamp where the sky doesn't reach.** A point light hangs 1 m behind and 0.4 m above the eyes. It comes up as daylight goes. Near faces then show brighter than far ones, so depth shows, such as a drop beyond a ledge. It casts no shadows: from beside the eyes, they would hide behind what casts them.
  - Tried at 200,000 lm, range 40 m, in a near-white warm. It is set to 0 lm, off, for now.

## Seams and tests

Test the pure functions inside the game crate, with `#[cfg(test)]` modules next to them:

1. **Padding assembly.** A Cell on a neighbour Chunk's border lands at the right padded index, and a missing neighbour leaves zeros.
2. **Quad conversion.** A single solid Cell produces 6 faces with outward normals. Two adjacent Cells of the same Element produce no faces between them.
3. **Colour mapping.** A quad of Element E gets E's colour.

Added at the same seam, in `src/render/mesher.rs`:

- liquid and gas Cells are left out of the padding;
- a face against a neighbour Chunk shows only if that neighbour is missing;
- a Chunk with no solids has no geometry.

Every quad test also checks each triangle's winding.

## Done when

- [x] The tests pass: 8 in `src/render/mesher.rs`.
- [x] **Human check:** `cargo run` shows rolling terrain with Dirt on top of Granite in flat colours, and caves are visible where hills are cut away. The frame rate stays smooth while Chunks load.
  - Checked by the agent with screenshots from a temporary harness, since removed.
  - What they showed: rolling Dirt hills, with Granite where caves break the surface. The cut-away side of render distance showed Granite under Dirt, full of caves.
  - Frame times are under *Decisions made during implementation*. The human hasn't checked it yet.

## Scope boundary

This step ends with solids on screen. Walking around comes in step 11.
