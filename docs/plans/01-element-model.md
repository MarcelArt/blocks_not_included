# Step 01: Element model and the Element data file

> Read `docs/plans/CONVENTIONS.md` first.

**Prerequisites:** step 00 (heat conduction) is done.

## Goal

Elements carry everything later steps need: Phase, Density, colour and Phase Transition thresholds. The ten starter Elements live in `assets/elements.ron`, and the sim crate loads and validates that file.

## Starting point

- `crates/sim/src/element.rs` has:
  - `Element` with `name`, `specific_heat` and `conductivity`, plus builder methods;
  - `ElementTable`, with Vacuum built in at `ElementId::VACUUM`, `add`, and a crate-private `get`.
- `crates/sim/tests/heat.rs` builds Elements with `Element::new(..).specific_heat(..).conductivity(..)`. Those tests must keep passing without edits.

## Design

- **Phase**: `pub enum Phase { Solid, Liquid, Gas, Vacuum }`. The built-in Vacuum is the only Element in the vacuum Phase (see `CONTEXT.md`). `Element::new` defaults to `Solid`.
- **New `Element` fields**, each with a builder method:
  - `phase`
  - `density`: kg/m³. It decides which Element sinks beneath another. A full liquid Cell holds `density × 1 m³`. It is also the default Mass for a placed solid.
  - `color`: `[f32; 3]`, linear RGB, the flat render colour.
  - `low: Option<Transition>` and `high: Option<Transition>`, where `pub struct Transition { pub temperature_k: f32, pub into: String }`. Below `low.temperature_k` the Element becomes `low.into`; above `high.temperature_k` it becomes `high.into`. Step 04 makes this happen; this step only stores and validates the thresholds.
- **`ElementId::index() -> u16`**: public, for renderers that need a compact number.
- **`ElementTable` public API**:
  - `get(id) -> &Element`
  - `id_of(name) -> Option<ElementId>`
  - `iter()`, yielding `(ElementId, &Element)`
  - `validate() -> Result<(), ElementError>`
  - `from_ron(text: &str) -> Result<ElementTable, ElementError>`, which parses and then validates.
  
  `Simulation::new` keeps accepting an unvalidated table, so the heat tests stay valid.
- **`ElementError`**: an enum with one variant per validation rule. Each variant names the offending Element in its `Display` output.
- **Dependencies**: `sim` may depend on `serde` (with `derive`) and `ron`.

### Validation rules

1. Names are unique and non-empty. `"Vacuum"` is reserved: the file never lists it.
2. Every Transition target exists.
3. A `low` target is a more condensed Phase (gas → liquid or solid, liquid → solid). A `high` target is a less condensed Phase.
4. **Hysteresis gap**: if A's `high` turns it into B at T, then B's `low`, if any, must be below T. Likewise, if A's `low` turns it into B at T, then B's `high`, if any, must be above T. Within one Element, `low` must be below `high`. Without this gap, a Cell sitting at the threshold would flip back and forth every Tick.
5. Every non-Vacuum Element has `density > 0`, `specific_heat > 0` and `conductivity >= 0`.

### File format (`assets/elements.ron`)

```ron
(
    elements: [
        (
            name: "Water",
            phase: Liquid,
            color: (0.15, 0.35, 0.8),
            density: 1000.0,
            specific_heat: 4.179,
            conductivity: 0.609,
            low: Some((temperature_k: 272.15, into: "Ice")),
            high: Some((temperature_k: 373.15, into: "Steam")),
        ),
    ],
)
```

`low` and `high` may be omitted when they are `None`.

### Starter Elements

Vacuum is built in and is not listed in the file.

| Name | Phase | Density kg/m³ | Specific heat J/(g·K) | Conductivity W/(m·K) | low | high | Colour |
|---|---|---|---|---|---|---|---|
| Oxygen | Gas | 1.43 | 1.005 | 0.024 | — | — | (0.75, 0.85, 1.0) |
| Carbon Dioxide | Gas | 1.98 | 0.846 | 0.0146 | — | — | (0.45, 0.45, 0.45) |
| Hydrogen | Gas | 0.0899 | 14.3 | 0.18 | — | — | (1.0, 0.75, 0.85) |
| Steam | Gas | 0.6 | 2.08 | 0.025 | 372.15 K → Water | — | (0.95, 0.95, 0.95) |
| Water | Liquid | 1000 | 4.179 | 0.609 | 272.15 K → Ice | 373.15 K → Steam | (0.15, 0.35, 0.8) |
| Magma | Liquid | 2600 | 1.0 | 1.0 | 1400 K → Igneous Rock | — | (1.0, 0.35, 0.05) |
| Ice | Solid | 917 | 2.05 | 2.18 | — | 273.15 K → Water | (0.75, 0.9, 1.0) |
| Granite | Solid | 2700 | 0.79 | 2.8 | — | 1500 K → Magma | (0.55, 0.5, 0.5) |
| Dirt | Solid | 1500 | 0.8 | 0.5 | — | — | (0.45, 0.3, 0.18) |
| Igneous Rock | Solid | 2900 | 1.0 | 2.0 | — | 1500 K → Magma | (0.25, 0.22, 0.22) |

## Seams and tests

**Seam:** `ElementTable`, through `from_ron`, `validate`, `id_of` and `get`. Put the tests in `crates/sim/tests/elements.rs`, in this order:

1. The shipped `assets/elements.ron` (loaded with `include_str!`) parses and validates, and contains all ten starter names.
2. `get(id_of("Water"))` returns Density 1000 and Phase Liquid.
3. A Transition that names an unknown Element is rejected.
4. A file that lists `"Vacuum"` is rejected.
5. Duplicate names are rejected.
6. A missing hysteresis gap is rejected. Example: Water boils at 373.15 K into Steam, and Steam condenses at 374 K into Water.
7. A wrong-direction Transition is rejected. Example: Water's `low` turns it into Steam.
8. A non-Vacuum Element with zero Density or zero specific heat is rejected.

## Done when

- [ ] All eight tests exist and pass, and `heat.rs` passes unchanged.
- [ ] `assets/elements.ron` holds exactly the table above.

## Scope boundary

This step ends once Elements are described, loaded and validated. Cells change Element in step 04, and hot reload arrives in step 15.
