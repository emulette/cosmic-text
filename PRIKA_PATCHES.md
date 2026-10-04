# Prika patches to cosmic-text 0.19.0

Upstream base: `c24886c2471e5606587c46090cd25dbbf209186b`.

- **Shaping uses the configured BCP 47 locale.** The fallback iterator exposes the
  shaping locale to HarfRust so language-specific OpenType substitutions follow
  the project locale. `Script` is re-exported for the engine fallback selector.

- **Missing static bold faces are synthesized per resolved glyph.** `src/font/mod.rs` applies
  Blink's 600 threshold after family matching; variable weight axes and families with a real
  bold face retain their existing selection. `src/swash.rs` strokes and fills the outline using
  Skia's size-dependent fake-bold width, preserving DirectWrite/Skia advances. The existing
  requested weight in `CacheKey` separates plain and synthesized images in every glyph atlas.

- **Project fallback preserves family order and matches faces within each family.** When
  `database_order_fallback` is enabled, `src/font/system.rs` keeps each family's first database
  position and promotes the face selected by `fontdb::Database::query` within that family.
  A bundled Regular face must not hide the same family's Bold face merely because it was loaded
  first. Equally eligible remaining faces retain source order. Platform fallback is unchanged.

- **Tracking is applied once per HarfRust cluster.** `src/shape.rs` adds the span's spacing
  to an advancing glyph of each cluster, retaining zero advances on attached marks. Subsequent
  glyph offsets in HarfRust order compensate for that added advance, preserving internal mark
  attachment in both paragraph directions. Ligatures remain one shaped cluster. A standalone
  zero-advance cluster remains eligible, including Prika's synthetic Ruby leading spacing slot.
  The owning engine-text behavior test covers positive and negative Hebrew mark tracking;
  Ruby column compensation counts clusters rather than glyphs.

- **Letter spacing is not applied after an invisible tracking control.** `src/shape.rs`
  (`is_tracking_control`) exempts `U+2060 WORD JOINER`, `U+2068 FIRST STRONG ISOLATE`, and
  `U+2069 POP DIRECTIONAL ISOLATE` from the per-cluster `letter_spacing` advance, so a zero-width
  control never widens the run it precedes. Upstream adds the span's letter spacing to every glyph
  advance without exception.

  Prika's ruby layout depends on this. `crates/engine-text/src/ruby.rs` wraps each ruby base in
  exactly those three characters, and `shape_buffer_with_column_spacing` in
  `crates/engine-text/src/layout.rs` distributes a ruby column's extra width as per-cluster letter
  spacing over the base run; without the exemption the invisible controls would each absorb one
  share and the base would no longer be centred under its reading. Any upgrade of this fork must reapply the patch or move the exemption into Prika by splitting the spans so the
  control characters carry no letter spacing of their own.

- **Glyph baselines round to the nearest pixel row.** `src/layout.rs` (`LayoutGlyph::physical`)
  rounds the vertically hinted baseline where upstream truncates it. Truncation painted every line
  up to a whole pixel above its layout position, while browsers paint an unrotated baseline on the
  nearest row; Prika's Runtime UI reproduces web designs, and its line boxes follow theirs.

- **A line centers whole-pixel ascent and descent.** `src/buffer.rs` (`LayoutRunIter`) rounds the
  line's font ascent and descent before centering them in the line height, as browsers lay out
  font metrics, so a baseline sits where a web line box puts it. Line heights are unchanged.

- **Fallback glyphs keep the lead font's ascent and descent.** `src/shape.rs` (`shape_run`) gives
  every glyph of a run the vertical metrics of the run's first font, where upstream gives a
  fallback glyph those of its own font. A line drawn only by a fallback font with other metrics,
  such as a symbol from Noto Sans Symbols 2, keeps the lead font's baseline, as a web inline box
  keeps its first available font's; line heights are unchanged.
