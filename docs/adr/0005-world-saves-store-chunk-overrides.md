# World saves store complete Chunks that differ from generated terrain

A World save combines its World Seed and generation rules with complete Cell data for Chunks that differ from the generator. It records the simulation bookkeeping needed to continue at the same Tick boundary. A wholly excavated Chunk can override the generator with Vacuum.

An edit log misses changes from heat, flow, and Phase Transitions. Storing every generated Chunk would make save size grow with terrain seen only at render distance. Using the generator as a baseline keeps that terrain cheap, at the cost of requiring a compatible generator to reopen the World. Development saves preserve the generator's startup definitions and the last accepted live rules independently, because a hot reload can cause them to diverge.
