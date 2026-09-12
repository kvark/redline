# Redline — design north star

**Original futuristic planet-circuit racing** — F-Zero / Wipeout energy, not a
licensed remake.

## Pillars

- **Physics-first ribbon racing** on procedurally generated planetary circuits.
- **Mars is the flagship** circuit; other worlds are championship variants of the
  same ribbon fantasy (tight moonlets, wide basins, volcano climbs).
- **Craft selection as race fantasy** — named machines in a championship garage,
  not a car catalog.
- **Native + web parity** — lavapipe / Vulkan and WASM WebGL2 should feel like
  the same race (fixed physics step, shared lighting story).

## Non-goals

- Remaking or pastiching any specific commercial IP frame-for-frame.
- Soft arcade hover that ignores the sphere and the ribbon.

## Menu / UX

Pre-race flow should read as entering a circuit: pick **Craft**, pick **Circuit**,
**Enter Circuit**. Scripts and smoke tests skip the menu for automation.

## Craft handling

Menu crafts are not cosmetics only: each `VehicleId` kit scales drive feel off the
Vector Spear baseline in `assets/vehicle.ron` (drive factor, motor force, mass,
wheel friction, lateral grip, jump). `RaceFuture` / Vector Spear keeps `kit =
None` so smoke scripts and the default garage pick stay on the ron numbers. AI
that reuse `KIT_*` share the same personality as the matching player craft.

