# Blocks Not Included

A first-person voxel sandbox set in an infinite world. Every Cell of matter in it is physically simulated: heat, gas, liquid and Phase Transitions, in the spirit of Oxygen Not Included.

## Language

### Matter

**Cell**:
A 1 m cube of the world grid together with the matter in it. Every Cell holds exactly one Element, with a Mass and a temperature.
_Avoid_: voxel, block, tile

**Element**:
A kind of matter with fixed physical properties and exactly one Phase, e.g. Oxygen, Water or Granite.
_Avoid_: material, substance, block type

**Phase**:
Whether an Element is a solid, a liquid, a gas or vacuum. Ice, Water and Steam are three distinct Elements, not one Element in three Phases. Vacuum is the only Element in the vacuum Phase.
_Avoid_: state

**Vacuum**:
The Element of a Cell that contains no matter. It always has zero Mass. A neighbouring gas or liquid can move part of its Mass into a Vacuum Cell, which then becomes that Element.
_Avoid_: air, empty, nothing

**Mass**:
How much of its Element a Cell holds. The smallest amount of matter is one gram. For a gas, Mass is also the Cell's pressure.
_Avoid_: amount, quantity, density

**Density**:
A fixed property of each Element that decides which Element sinks beneath another, and how much Mass a full liquid Cell holds. It never depends on how much Mass a Cell currently holds.
_Avoid_: weight, heaviness

**Capacity**:
The most Mass one liquid Cell holds: its Element's Density × 1 m³, so 1,000,000 g for Water. Only a Cell that has just become a liquid through a Phase Transition can briefly hold more.
_Avoid_: max mass, volume

**Phase Transition**:
One Element becoming a different Element when its temperature crosses a threshold that Element defines, e.g. Water above 100 °C becomes Steam. An Element that defines no threshold never transitions.
_Avoid_: state change, phase change

**Displacement**:
A gas or liquid moving part of its Mass into a neighbouring Cell of a different gas, once that gas has left entirely for one of its own neighbours holding the same gas, or for Vacuum. It happens only from a Cell holding more Mass, and never when the displaced gas has nowhere to go.
_Avoid_: pushing aside, shoving

### World

**Chunk**:
A fixed-size cube of Cells. It is the unit in which the world is generated, loaded and simulated.

**Active Region**:
The box of Chunks around the player that is simulated. Its boundary is sealed and insulated, so no Mass or heat crosses into a Dormant Chunk.
_Avoid_: simulation bubble, sim radius, loaded area

**Dormant Chunk**:
A Chunk outside the Active Region. Its time is stopped, and when it rejoins the Active Region it resumes exactly as it was left.
_Avoid_: frozen chunk, sleeping chunk, unloaded chunk

**Settled Chunk**:
A Chunk inside the Active Region in which nothing changed during the last Tick. It skips work until something nearby changes. Unlike a Dormant Chunk, its time keeps running.
_Avoid_: quiet chunk, idle chunk

**Atmosphere**:
The gas above the terrain. It is made of ordinary gas Cells and simulated like any other. Gas that rises past the top of the world is lost.
_Avoid_: air, sky, ambient

### Simulation

**Tick**:
One step of the simulation on its fixed clock. Ticks are independent of the frame rate.
_Avoid_: frame, update

**Overlay**:
A view the player can toggle that colours Cells by a single property (Element, temperature or Mass) in place of their normal appearance.
_Avoid_: lens, vision mode
