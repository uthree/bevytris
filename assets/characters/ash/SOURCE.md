# How this pack's art was made

Generated for bevytris by `scripts/make_character_art.sh`.

- Method: hand-supplied hero images
- Source directory: `/var/folders/rp/3pkr8jjx5276_vpcxyf1c3cw0000gn/T/tmp.LH5jlsmNOp` (not part of this repository)
- Background removal: BiRefNet_toonout, via ComfyUI
- Derived: standing_left is a mirror of standing_right; anything absent above
  was derived from standing.png by `scripts/make_character_art.sh`.

## Licence: this repository's

The characters here are nobody else's. Open-weight image models license
their *weights*, not their *outputs* — the Fair AI Public License and
CreativeML OpenRAIL both state that outputs are not covered and that no
contributor claims rights in them — and only outputs are redistributed.

The voices in `voices/` are a separate matter with separate terms: audio
generated from a VOICEVOX voice library may be used commercially and
non-commercially provided the character is credited. See `metadata.json`'s
`author` field for the credit this pack carries.
