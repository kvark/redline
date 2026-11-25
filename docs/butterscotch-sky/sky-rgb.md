# On-screen sky RGB (sRGB, 1280×720 captures)

Native: lavapipe, `--script race --freeze-at 5.9`, the same Mars corner.

| sample (x, y) | before (main 98b0ab4) | after |
|---|---|---|
| sky over the gate (620, 100) | (179, 146, 90) | (170, 101, 33) |
| sky, right of the gate (820, 60) | (180, 147, 95) | (174, 105, 42) |

Track, gates, pads and liveries are unchanged. The env map only feeds the sky pass:

| sample | before | after |
|---|---|---|
| boost pad (560, 255) | (88, 101, 114) | (88, 101, 114) |
| gate arch (470, 110) | (118, 69, 54) | (118, 69, 54) |
| red livery (600, 450) | (202, 190, 173) | (202, 190, 173) |
| purple livery (900, 290) | (210, 200, 184) | (210, 200, 184) |
| track (400, 600) | (203, 162, 125) | (203, 162, 125) |

Web: headless Chrome, SwiftShader WebGL2, `?script=race`, at 24 s wall time. Same pose in both captures.

| sample (fraction of frame) | before | after |
|---|---|---|
| (0.40, 0.05) | (100, 59, 25) | (81, 23, 4) |
| (0.62, 0.04) | (99, 58, 23) | (80, 22, 4) |

The web frame as a whole, sky and terrain alike, lands at the native tone-mapped value *without* the final sRGB encode. That gap existed before this change (before: web (100, 59, 25) vs native (179, 146, 90)) and comes from the Blade WebGL present path, not from the sky bake. Relative to the rest of the web frame, the sky moves the same way it does natively.
