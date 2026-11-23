# Bevy 0.20 notes for game steps

The workspace uses **Bevy 0.20.1**, released in October 2026. Most tutorials and snippets online target older versions. Check every Bevy API you write against one of two sources: Context7 (`/bevyengine/bevy/v0.20.1`), or the examples downloaded with the crate under `~/.cargo/registry/src/*/bevy-0.20.1/examples/`. The migration guides live at <https://bevy.org/learn/migration-guides/0-19-to-0-20/> and `…/0-18-to-0-19/`.

## Crate versions known to work with Bevy 0.20 (checked 2026-11-22)

| Crate | Version | Note |
|---|---|---|
| `bevy_egui` | 0.43 | uses egui 0.36 |
| `bevy-inspector-egui` | 0.38 | needs bevy_egui 0.43 |
| `binary-greedy-meshing` | 0.6.4 | engine-agnostic; see below |

## `binary-greedy-meshing`

- Its README says only 62³ chunks are supported. That is out of date: 0.6.4 has `Mesher<const CS: usize>`, so `Mesher::<32>` fits our 32³ Chunks.
- Input is a padded `(CS+2)³` array of `u16`, holding one layer of neighbour Cells on every side. The value 0 means empty.
- Read the crate source (`pad_linearize`, `MeshData`/`Mesher`, and the `quads` encoding) before you rely on index order. It is not the same as `sim`'s Chunk order.

## APIs that changed recently

- **Render graph (0.19):** render-graph nodes became ordinary systems in the `RenderGraph` schedule. Use `StandardMaterial` with vertex colours wherever it is enough, and avoid custom render passes.
- **Shaders (0.20):** shaders that use imports or defines must be **WESL** (`.wesl`), with `import a::b;` and `@if(X)`. GLSL was removed. Plain WGSL without directives still works.
- **Meshes:**
  - `Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)` drops the CPU copy after upload.
  - Spawn meshes with `Mesh3d` and `MeshMaterial3d`.
  - `MeshAabb::compute_aabb` was renamed to `get_aabb`.
- **Translucency:** 0.20 adds `Material::enable_oit` (order-independent transparency) for overlapping translucent faces.
- **Extraction:** the `ExtractComponent` and `ExtractResource` derives now need `#[extract_app(RenderApp)]`.
- **Buffered events:** these are `Message`s (`MessageWriter` / `MessageReader`), separate from observer `Event`s. Confirm against 0.20 before using them.

## Found in step 10

- **Ambient light** is the `GlobalAmbientLight` resource. `AmbientLight` is now a per-camera component that overrides it.
- **Free camera.** `bevy::camera_controller::free_camera::{FreeCamera, FreeCameraPlugin}` needs Bevy's `free_camera` feature. Step 10 enabled it for its temporary camera.
- **Frame-time measurements.** The default `WinitSettings::game()` caps an unfocused window at 60 Hz. Insert `WinitSettings::continuous()` before measuring, or a window without focus looks slow.
- **Light levels.** See step 10's lighting decision for values that keep flat colours readable under the default exposure.

## Threads and tasks

- On this 8-thread laptop, Bevy's defaults are 4 compute threads, 2 async-compute threads and 2 IO threads. The sim runs on its own thread (ADR 0003) with its own sized pool (step 08), so leave Bevy's pools at their defaults unless you have a measurement that says otherwise.
- For off-frame work, use `AsyncComputeTaskPool::get().spawn(async move { … })`, then poll the result with `bevy::tasks::futures::check_ready(&mut task)`.

## Running on this laptop

- It has hybrid graphics (Intel plus a GTX 1650). If the adapter Bevy logs at startup is the Intel GPU, run with `prime-run cargo run`, or set `WGPU_ADAPTER_NAME` to force the NVIDIA one.
- Hot reload of assets needs Bevy's `file_watcher` feature (step 15).
