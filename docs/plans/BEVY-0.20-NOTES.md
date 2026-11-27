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

## Found in step 11

- **Cursor capture.** `CursorOptions` is a component of its own on the window entity, not a field of `Window`. Use `Single<&mut CursorOptions, With<PrimaryWindow>>` and set `grab_mode` and `visible`. On X11, `CursorGrabMode::Locked` falls back to `Confined`.
- **Mouse look.** The `bevy::input::mouse::AccumulatedMouseMotion` resource holds the whole movement since the last frame, in `delta`. Don't scale it by time.
- **`Single` that stops matching.** Once its filter stops matching, a system taking `Single<…>` is skipped without a warning. Step 11 relies on this for its `With<AwaitingSpawn>` spawn system.
- **The `free_camera` feature is gone,** along with step 10's temporary camera.
- **Agent checks in a real window.**
  - A system in `PreUpdate`, ordered `.after(bevy::input::InputSystems)`, can `press` and `release` keys on `ButtonInput<KeyCode>`. Every game system then sees them as held.
  - `commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path))`, from `bevy::render::view::screenshot`, saves a frame.
- **Vulkan loader errors.** Two `ERROR wgpu_hal::vulkan::instance … liblsfg-vk-layer.so` lines at startup come from a Vulkan layer installed on this laptop, not from the game. They are harmless.

## Found in step 12

- **`bevy_egui` 0.43.1** (egui 0.36.2) is in, with default features off except `render` and `default_fonts`. Add `EguiPlugin::default()` once, in `main.rs`; it attaches itself to the player's camera.
  - Draw UI from a system in the `EguiPrimaryContextPass` schedule that returns `Result` and calls `contexts.ctx_mut()?`.
  - egui 0.36 names: `egui::Panel::left/right/top/bottom` for side panels, and `Slider::from_get_set(range, |value| …)`, which shows one unit (°C) while storing another (K).
- **Who has the pointer.** `bevy_egui::input::EguiWantsInput` is written in `PostUpdate`, so systems in `Update` see the previous frame's value. Its `wants_any_pointer_input()` is already true while the pointer only hovers over a window.
- **Captured cursor and egui.** On X11 the hidden cursor still moves, and egui would let it hover over and press a window. Draw windows with `.interactable(false)` while the cursor is captured: egui's hit-testing skips them, so egui never wants the pointer.
- **Using up a click.** `ButtonInput::clear_just_pressed(button)` hides the press from systems that run later in the frame. `capture_cursor` uses it.
- **Gizmos.** `gizmos.cube(transform, color)` and `gizmos.sphere(Isometry3d::from_translation(centre), radius, color)`, drawn from `Update`.
- **Agent checks.** Presses made on `ButtonInput` don't reach egui, which reads input messages. In a window without focus, `Window::cursor_position()` is `None`.

## Found in step 13

- **Translucent `StandardMaterial`.**
  - With `AlphaMode::Blend`, vertex colours multiply the base colour, alpha included. A white base therefore lets each vertex's alpha set its opacity.
  - Blend pipelines don't write depth, so every overlapping face in a mesh shows, blended in index order.
  - Blend meshes still cast shadows: the shadow pass treats them as `MAY_DISCARD`. Add `bevy::light::NotShadowCaster` to stop that.
  - `cull_mode: None` with `double_sided: true` shows a face from both sides, lit correctly from each.
- **OIT** needs `OrderIndependentTransparencySettings` on the camera. `Material::enable_oit` already defaults to `true`, so `StandardMaterial` takes part once the camera has the setting.
- **Measuring frame time on this laptop.**
  - The Steam client can take a whole core in the background, and the laptop slows as it warms up, so runs drift over time. Build the A and B versions as separate binaries, and alternate their runs.
  - To run a built example outside `cargo run`, set `LD_LIBRARY_PATH` to `target/debug/deps` plus the toolchain's `lib/rustlib/x86_64-unknown-linux-gnu/lib` (`dynamic_linking` needs both), and `BEVY_ASSET_ROOT` to a folder holding `assets/`.
- **Dev profile.** The game crate is built at opt-level 1, where a call to a non-generic function from another crate isn't inlined unless that function is `#[inline]`. Mark any small `sim` function the game calls for every Cell `#[inline]`.

## Found in step 14

- **Run conditions.** The combinator is `or_else`, not `or`: `resource_changed::<A>.or_else(resource_changed::<B>)`. It short-circuits, so the second condition doesn't run, or update its change ticks, on frames when the first holds.
- **Hiding meshes.**
  - `Mesh3d` requires `Visibility` (the camera's visibility plugin registers it), so every mesh entity has one to set.
  - To hide entities spawned during `Update` before they are drawn, set `Visibility` in `PostUpdate`, ordered `.before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate)`.
  - A `Visibility::Hidden` mesh casts no shadow.
- **Translucent meshes that show where a value changes.**
  - `binary-greedy-meshing`'s `fast_mesh` with an all-zero opaque mask, and every non-zero value in the transparent mask, gives faces wherever the value changes, on both sides, merged greedily.
  - Keep back faces culled: from any side, only the face pointing at the camera shows.
- **egui.**
  - `ctx.viewport_rect()` is the whole window; `content_rect()` leaves out safe-area insets.
  - `ctx.layer_painter(egui::LayerId::new(egui::Order::Background, id))` paints over the 3D scene but under egui's windows.
  - `egui::Rgba::from_rgb` takes linear colours, and `Color32::from(rgba)` converts them for display.
- **Agent checks.** `Query<(), (With<Player>, Without<AwaitingSpawn>)>::single().is_ok()` tells a harness when the player has spawned. Allow about 30 s after spawn before measuring frame times: render distance is still loading and meshing before then, and frames run 20–25 ms.

## Found in step 15

- **Hot reload.** The game crate's default `dev` feature turns on `bevy/file_watcher`, so `cargo run` watches `assets/`. The watcher waits 300 ms for a burst of file events to end before reloading.
- **Two loaders for one extension.** Both simulation files are `.ron`. Their loaders claim no extension, and `AssetServer::load::<A>(path)` then chooses the loader by asset type alone.
- **Failed reloads.**
  - A reload that fails writes an `AssetLoadFailedEvent<A>` message, and the asset keeps its previous value in `Assets<A>`. The next successful reload writes `AssetEvent::Modified`, not `Added`.
  - To show the loader's own message, without the server's wrapping (which names the loader's type), match `AssetLoadError::AssetLoaderError(e)` and call `e.error().downcast_ref::<YourError>()`.
  - The asset server also logs its own `error!` for every failed reload. It then retries the load without the asset type, which a loader that claims no extension can't serve, and logs `Could not find an asset loader matching: Asset Type: None`. Both are harmless.
- **Messages in tests.** To read a game `Message` from a test that drives `app.update()`, add a system that copies it into a resource: messages are cleared after two frames.
- **egui.** `ui.add_enabled(enabled, egui::Button::new(label))` greys out a button.

## Found while relighting caves (after step 15)

- **Sky light that depends on the normal.** An `EnvironmentMapLight` on the camera lights every PBR material by its surface normal.
  - `EnvironmentMapLight::hemispherical_gradient(images, top, mid, bottom)` builds one with the same shade on all four sides.
  - For a different shade per side, build a 1×1 `Image` with 6 layers and `texture_view_descriptor: Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), .. })`. Its layers are in wgpu's order: +x, −x, +y, −y, +z, −z. `Rgba8Unorm` texels work.
  - Diffuse light is about texel × `intensity`, on the same scale as `GlobalAmbientLight::brightness`.
  - `GlobalAmbientLight` still adds on top, so set it to `GlobalAmbientLight::NONE`.
- **Texture types** (`Extent3d`, `TextureDimension`, `TextureFormat`, `TextureViewDescriptor`, `TextureViewDimension`) are under `bevy::render::render_resource`.
- **Point lights.** The shadow switch is `PointLight::shadow_maps_enabled`. `EntityCommands::with_child(bundle)` hangs one from the camera.

## Found in step 18

- **States.**
  - `StatesPlugin` (in `DefaultPlugins`; add `bevy::state::app::StatesPlugin` to `MinimalPlugins` in tests) puts the `StateTransition` schedule after `PreUpdate`. A `NextState` set in `PreUpdate` therefore takes effect before `Update` in the same frame. One set during `Update` waits for the next frame.
  - `init_state` enables state-scoped entities by itself. Spawn with `DespawnOnExit(State::Variant)`.
  - Sub-states: `#[derive(SubStates, …)]` with `#[source(Parent = Parent::Variant)]`, then `app.add_sub_state::<S>()`. `State<S>` exists only while the source state matches. Exit schedules run leaf first, enter schedules root first.
  - `NextState` is an enum, `Unchanged | Pending(S) | PendingIfDifferent(S)`, so a system can read a transition requested earlier in the frame. `set` runs the transition schedules even when the state stays the same; `set_if_different` doesn't.
  - `OnTransition { exited, entered }` exists.
- **Run conditions and missing resources.** The executor evaluates run conditions before validating the system's parameters. A system gated by `in_state` can therefore take a resource that exists only in that state.
  - All of a system's conditions, from separate `run_if` calls and from its sets, are evaluated every frame without short-circuiting.
  - A condition that takes a missing `Res` panics through the default error handler. Use `resource_exists_and_changed` or `in_state(…).and_then(…)`, which does short-circuit. (The combinator is `and_then`, like `or_else`; there is no `and`.)
- **Commands.** `impl Command for T` needs `type Out = ();`. From a test, `app.world_mut().commands().queue(c)` then `app.world_mut().flush()`.
- **egui's primary context** goes only to the first camera ever spawned: `setup_primary_egui_context_system` remembers in a `Local`. A camera spawned later, as when a World reopens, needs `PrimaryEguiContext` inserted by hand. Set `EguiGlobalSettings { auto_create_primary_context: false, .. }` so no other camera takes it first.
- **Asset events** (`AssetEvent<A>`) are written in `PostUpdate`. A system in `Update` reads events from the previous frame's changes.
- **Headless test apps.**
  - The god tools' `Gizmos` need `bevy::gizmos::GizmoPlugin`. That plugin also needs `init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()`.
  - Chunk meshes need `AssetPlugin`, `init_asset::<Mesh>()` and `init_asset::<StandardMaterial>()`.
  - A camera with `PrimaryEguiContext` spawns without egui's plugin.
  - `InputPlugin` turns `KeyboardInput` and `MouseButtonInput` messages into `ButtonInput` presses. Its `window` field can be any entity.
- **Input.** `ButtonInput::reset_all()` clears pressed, just pressed and just released. A button still physically held then reads as up until it is pressed again, and its release sends no `just_released`. `Window::focused` defaults to `true`.

## Found in step 19

- **The window's close button.** `WindowPlugin { close_when_requested: false, .. }` leaves `WindowCloseRequested` messages to the game: nothing closes until it writes `AppExit`. A plugin that reads them should `add_message::<WindowCloseRequested>()` itself, so headless tests (no `WindowPlugin`) can send them. `App::should_exit()` reports an `AppExit` written in the last two frames.
- **A clock for tests.** `TimeUpdateStrategy::ManualDuration(d)` moves `Time<Real>` by exactly `d` each `app.update()`. `Time<Virtual>` (the plain `Time`) clamps each step to `max_delta`, 250 ms by default, so count play time in `Time<Real>`. Set it to `Duration::ZERO` while a test waits in real time, so play time doesn't creep.
- **egui 0.36.**
  - A button's label follows its parent `Ui`'s layout: inside `ui.vertical_centered(…)`, a wide button's label is centred; otherwise it sits on the left.
  - `CentralPanel::show` now takes a `Ui`, not a `Context`. A centred `egui::Window` with `.title_bar(false)` and `.anchor(Align2::CENTER_CENTER, …)` works as a full-screen menu's page.
  - `egui::Modal::new(id).backdrop_color(…).show(ctx, …)` dims everything behind it and takes the pointer. Its response's `should_close()` can be ignored.
  - `TextEdit::id(id)` plus `ui.memory_mut(|m| m.request_focus(id))` focuses a field that only appears next frame.
  - One-line text that may run long: `LayoutJob::single_section(text, TextFormat::simple(font, color))` with `job.wrap = TextWrapping::truncate_at_width(w)`, then `painter.layout_job(job)` and `painter.galley(pos, galley, color)`.
- **Agent checks of egui menus in a real window.** bevy_egui reads the pointer from `WindowEvent` messages, so a harness can click: write `WindowEvent::CursorMoved(CursorMoved { window, position, delta: None })`, then `WindowEvent::MouseButtonInput` pressed and released, in logical pixels. Typing is `KeyboardInput` with `logical_key: Key::Character(…)`; a space must be `Key::Space`.
- **Math.** `Quat::angle_between` can't tell rotations less than about 7e-4 rad apart from equal in `f32`: its `acos` of a dot product near 1 loses precision.

## Threads and tasks

- On this 8-thread laptop, Bevy's defaults are 4 compute threads, 2 async-compute threads and 2 IO threads. The sim runs on its own thread (ADR 0003) with its own sized pool (step 08), so leave Bevy's pools at their defaults unless you have a measurement that says otherwise.
- For off-frame work, use `AsyncComputeTaskPool::get().spawn(async move { … })`, then poll the result with `bevy::tasks::futures::check_ready(&mut task)`.

## Running on this laptop

- It has hybrid graphics (Intel plus a GTX 1650). If the adapter Bevy logs at startup is the Intel GPU, run with `prime-run cargo run`, or set `WGPU_ADAPTER_NAME` to force the NVIDIA one.
- Hot reload of assets needs Bevy's `file_watcher` feature, which the default `dev` feature turns on (step 15). Build a release with `--no-default-features`.
