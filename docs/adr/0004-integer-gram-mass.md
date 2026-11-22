# Mass is stored as whole grams

A Cell's Mass is an integer number of grams, not a floating-point number of kilograms. With integers, the simulation conserves Mass exactly, so "total Mass is unchanged after a Tick" can be enforced as a hard invariant in tests and on the in-game debug panel. Any flow smaller than 1 g rounds to zero, which lets gases and liquids actually come to rest instead of trickling forever. That in turn lets Settled Chunks skip work without approximating Mass.

## Considered Options

- **`f32` kilograms**: simpler arithmetic, but rounding slowly creates or destroys matter, and a cleanup rule is needed for tiny amounts. ONI deletes tiny masses, and players notice.

## Consequences

- The smallest amount of matter is 1 g. A single Cell can hold at most about 4,000 tonnes.
- Temperature stays floating-point (Kelvin), so energy is not conserved exactly. Only Mass is.
