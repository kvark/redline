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
**Enter Circuit**. Interactive starts hold the grid for a championship **3-2-1-GO**
countdown (drive locked until GO); Esc cancels back to the menu. Crossing the
line flashes lap time / **FINAL LAP** / **FINISH** callouts (visual only), then a
brief **results** board (place P1–Pn, race / lap times). **Rematch** (or Enter)
restarts with the same craft and circuit; Continue / Esc / auto-return go back
to the menu. Mid-lap gates track **sector** splits: the HUD shows current sector
time and best sector, with a brief **BEST SECTOR** flash on a personal best.
The race HUD also shows live **place**, ticking lap time, best/last, and LAP X/Y.
Scripts and smoke tests skip the menu, countdown, and results board for
automation; callouts never block drive.

## Craft handling

Menu crafts are not cosmetics only: each `VehicleId` kit scales drive feel off the
Vector Spear baseline in `assets/vehicle.ron` (drive factor, motor force, mass,
wheel friction, lateral grip, jump). `RaceFuture` / Vector Spear keeps `kit =
None` so smoke scripts and the default garage pick stay on the ron numbers. AI
that reuse `KIT_*` share the same personality as the matching player craft.

## Ribbon traction

On-ribbon lateral grip is stronger than runoff (physics-first stickiness, not soft
arcade hover). Leaving the ribbon far enough to scrub, then re-acquiring it, fires
a short forward boost pulse — championship recovery after a mistake, shared by
player and AI kits alike.


## Boost pad readability pass

The circuit already has a strong cyan ribbon, luminous gate markers and
planet-specific colors, but the old all-cyan pads asked the player to read a
small cyan detail inside that cyan visual system. The pad is now an original
procedural asset: a dark deck, four amber edge guides, and two cyan chevrons.
Amber is reserved for the footprint and cyan for the direction. Shape still
carries both cues without depending on color alone. All surfaces are generated
as one GLB and draped onto the same ribbon dimensions used for triggering. No
commercial artwork is copied or included.

Reference roles: [Nintendo's F-ZERO X manual](https://www.nintendo.com/eu/media/downloads/games_8/emanuals/nintendo_8/Manual_Nintendo64_FZeroX_EN.pdf)
is useful for separating track feedback from rules about limited boost power;
Redline does not add an energy meter in this pass.
[PlayStation's WipEout Omega Collection announcement](https://blog.playstation.com/2017/03/30/wipeout-omega-collection-coming-to-ps4-on-june-6th/)
is a visual reference for bold high-speed circuit cues and the importance of
speed laps as a legible mode, not a source of marks, logos, or assets.

The gameplay change is about making the cue truthful: a chassis crossing a
corner between two physics samples can get the same pulse as a craft sampled
inside it. The sweep respects the plate footprint and trigger height and does
not connect positions across a recovery teleport. Boost intensity, cooldown,
sector placement, and the manual controls are unchanged.
