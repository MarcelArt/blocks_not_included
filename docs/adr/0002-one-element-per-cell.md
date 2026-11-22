# One Element per Cell

Every Cell holds exactly one Element, with a Mass and a temperature, as in Oxygen Not Included. Different gases never share a Cell; they swap places by density instead. Each Phase is its own Element (Ice, Water, Steam), and Phase Transitions link them. Vacuum is an Element with zero Mass, so the rule has no exceptions. We chose this model because fixed-size Cells are cheap and easy to process in parallel, and because density layering makes the simulation easy to read.

## Considered Options

- **Mixture model** (several substances per Cell): gives realistic "air", but costs several times the memory and work per Cell and makes the visuals muddier.
- **Layered model** (a solid, plus a gas mixture, plus a liquid volume, per Cell): the most flexible option and the most expensive.

## Consequences

- There is no "air". A breathable Atmosphere is made of real gas Cells, such as Oxygen.
- For a gas, a Cell's Mass also serves as its pressure.
- A partially filled liquid Cell holds no gas in the rest of its volume.
- Phase Transitions happen in place: the Cell's Element changes and its Mass stays.
