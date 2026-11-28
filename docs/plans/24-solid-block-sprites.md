# 24 · Distinct solid block sprites

[Plan index](README.md) · Requires [22 Solid Element textures](22-solid-textures.md).

## Goal and starting point

Replace the artwork for all four shipped solid Elements in `assets/textures/solids.png`. One seamless 16 × 16 tile represents each material on every face. Water and Magma use their own liquid atlas from step 23.

`assets/elements.ron` defines the materials and colours. `src/render/mod.rs` loads the shared solid atlas. `src/render/mesher.rs` selects its band by Element name and applies vertex colour and corner shading. The existing UV layout already supports a different pattern for each block.

## Design

1. Use the Codex imagegen skill to generate four original, front-on, seamless pixel textures. Give Dirt irregular packed clumps with small pebbles, Granite interlocking coarse mineral grains, Ice broad clear-looking planes crossed by thin fractures, and Igneous Rock a dense surface broken by dark pits. Keep the lighting neutral so each face responds to the game's lighting.
2. Reduce each selected tile to game resolution and inspect a 3 × 3 repeat. Check the patterns after material tinting. Regenerate any tile whose identifying marks disappear at this scale.
3. Repack the final tiles in the order consumed by the mesher. This is the packing contract for `assets/textures/solids.png` (Python, using Pillow images):
   
   ```python
   atlas = Image.new("RGB", (16, 2192), "white")
   for band, tile in enumerate((dirt, granite, ice, igneous_rock)):
       assert tile.size == (16, 16)
       for row in range(34):
           atlas.paste(tile, (0, (34 * band + row) * 16))
   ```

## Tests and completion

- Run `cargo test --workspace`, including the UV, band-selection and white-fallback tests in `src/render/mesher.rs`. Inspect the packed PNG for 34 identical repeats per band and a final white 16 × 16 tile.
- Run `cargo run`. Place adjacent patches of all four solids, large enough to see several repeats on their tops and sides. Inspect at ordinary walking distance in sunlight and shade with diagnostic Overlays off. Capture both views, convert copies to grayscale, and identify each material by its stated pattern with the readout hidden.
- Done when all four can be identified in both grayscale views, the colour views look coherent, and the repeats show no join lines or neighbouring atlas bands.

## Implementation and verification

The four 16 × 16 tiles were generated with the ImageGen CLI. Ice received a hand-edited fracture highlight after downsampling so its thin lines remain visible at walking distance.

- `cargo test --workspace` passed, including the mesher tests. Inspection of the shipped PNG confirmed 34 identical copies per material band and a white 16 × 16 fallback tile.
- `cargo run` launched successfully. The solid-block inspection scene showed all four patches side by side with diagnostic Overlays off. In grayscale, Dirt's packed clumps, Granite's angular grains, Ice's fractures and Igneous Rock's pits remained distinguishable. No join lines or neighbouring bands were visible. [Colour capture](../../output/solid-sprites/materials.png) · [Grayscale capture](../../output/solid-sprites/materials-grayscale.png)
- **Pending** — The capture shows sunlit tops and shaded sides in one frame. The remaining acceptance check is to capture separate sunlight and shade views at walking distance and identify all four materials in each grayscale copy.
