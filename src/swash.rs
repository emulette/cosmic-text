// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec};
#[cfg(feature = "no_std")]
use core_maths::CoreFloat;

use core::fmt;
use swash::scale::{image::Content, ScaleContext};
use swash::scale::{Render, Source, StrikeWith};
use swash::zeno::{Format, Stroke, Vector};

use crate::{CacheKey, CacheKeyFlags, Color, FontSystem, HashMap};

pub use swash::scale::image::{Content as SwashContent, Image as SwashImage};
pub use swash::zeno::{Angle, Command, Placement, Transform};

fn swash_image(
    font_system: &mut FontSystem,
    context: &mut ScaleContext,
    cache_key: CacheKey,
) -> Option<SwashImage> {
    let Some(font) = font_system.get_font(cache_key.font_id, cache_key.font_weight) else {
        log::warn!("did not find font {:?}", cache_key.font_id);
        return None;
    };

    let variable_width = font
        .as_swash()
        .variations()
        .find_by_tag(swash::Tag::from_be_bytes(*b"wght"));

    // Build the scaler
    let mut scaler = context
        .builder(font.as_swash())
        .size(f32::from_bits(cache_key.font_size_bits))
        .hint(!cache_key.flags.contains(CacheKeyFlags::DISABLE_HINTING));
    if let Some(variation) = variable_width {
        scaler = scaler.normalized_coords(font.as_swash().variations().normalized_coords([(
            swash::Tag::from_be_bytes(*b"wght"),
            f32::from(cache_key.font_weight.0).clamp(variation.min_value(), variation.max_value()),
        )]));
    }
    let mut scaler = scaler.build();

    // Compute the fractional offset-- you'll likely want to quantize this
    // in a real renderer
    let offset = if cache_key.flags.contains(CacheKeyFlags::PIXEL_FONT) {
        Vector::new(
            cache_key.x_bin.as_float().round(),
            cache_key.y_bin.as_float().round(),
        )
    } else {
        Vector::new(cache_key.x_bin.as_float(), cache_key.y_bin.as_float())
    };

    // Select our source order
    let mut render = Render::new(&[
        // Color outline with the first palette
        Source::ColorOutline(0),
        // Color bitmap with best fit selection mode
        Source::ColorBitmap(StrikeWith::BestFit),
        // Standard scalable outline
        Source::Outline,
    ]);
    render
        // Select a subpixel format
        .format(Format::Alpha)
        // Apply the fractional offset
        .offset(offset)
        .transform(if cache_key.flags.contains(CacheKeyFlags::FAKE_ITALIC) {
            Some(Transform::skew(
                Angle::from_degrees(14.0),
                Angle::from_degrees(0.0),
            ))
        } else {
            None
        });
    let fill = render.render(&mut scaler, cache_key.glyph_id)?;
    let stroke_width = font.synthetic_bold_stroke_width(f32::from_bits(cache_key.font_size_bits));
    if stroke_width == 0.0 || !matches!(fill.source, Source::Outline) || fill.data.is_empty() {
        return Some(fill);
    }

    // Stroke the actual scaled outline, with Skia's default miter join/limit and butt cap.
    // Its bitmap bearings include the outward growth; the shaped advance is never modified.
    let stroke = render
        .style(Stroke::new(stroke_width))
        .render(&mut scaler, cache_key.glyph_id)?;
    // The fill's control-point bounds may exceed the stroke's evaluated curve bounds.
    let left = fill.placement.left.min(stroke.placement.left);
    let top = fill.placement.top.max(stroke.placement.top);
    let right = (fill.placement.left + fill.placement.width as i32)
        .max(stroke.placement.left + stroke.placement.width as i32);
    let bottom = (fill.placement.top - fill.placement.height as i32)
        .min(stroke.placement.top - stroke.placement.height as i32);
    let placement = Placement {
        left,
        top,
        width: (right - left) as u32,
        height: (top - bottom) as u32,
    };
    let mut image = SwashImage {
        source: Source::Outline,
        content: Content::Mask,
        placement,
        data: vec![0; placement.width as usize * placement.height as usize],
    };
    for layer in [stroke, fill] {
        let dx = (layer.placement.left - left) as usize;
        let dy = (top - layer.placement.top) as usize;
        for y in 0..layer.placement.height as usize {
            for x in 0..layer.placement.width as usize {
                let src = u32::from(layer.data[y * layer.placement.width as usize + x]);
                let dst = &mut image.data[(y + dy) * placement.width as usize + x + dx];
                *dst = (src + (u32::from(*dst) * (255 - src) + 127) / 255) as u8;
            }
        }
    }
    Some(image)
}

fn swash_outline_commands(
    font_system: &mut FontSystem,
    context: &mut ScaleContext,
    cache_key: CacheKey,
) -> Option<Box<[swash::zeno::Command]>> {
    use swash::zeno::PathData as _;

    let Some(font) = font_system.get_font(cache_key.font_id, cache_key.font_weight) else {
        log::warn!("did not find font {:?}", cache_key.font_id);
        return None;
    };

    let variable_width = font
        .as_swash()
        .variations()
        .find_by_tag(swash::Tag::from_be_bytes(*b"wght"));

    // Build the scaler
    let mut scaler = context
        .builder(font.as_swash())
        .size(f32::from_bits(cache_key.font_size_bits))
        .hint(!cache_key.flags.contains(CacheKeyFlags::DISABLE_HINTING));
    if let Some(variation) = variable_width {
        scaler = scaler.normalized_coords(font.as_swash().variations().normalized_coords([(
            swash::Tag::from_be_bytes(*b"wght"),
            f32::from(cache_key.font_weight.0).clamp(variation.min_value(), variation.max_value()),
        )]));
    }
    let mut scaler = scaler.build();

    // Scale the outline
    let mut outline = scaler
        .scale_outline(cache_key.glyph_id)
        .or_else(|| scaler.scale_color_outline(cache_key.glyph_id))?;

    if cache_key.flags.contains(CacheKeyFlags::FAKE_ITALIC) {
        outline.transform(&Transform::skew(
            Angle::from_degrees(14.0),
            Angle::from_degrees(0.0),
        ));
    }

    // Get the path information of the outline
    let path = outline.path();

    // Return the commands
    Some(path.commands().collect())
}

/// Cache for rasterizing with the swash scaler
pub struct SwashCache {
    context: ScaleContext,
    pub image_cache: HashMap<CacheKey, Option<SwashImage>>,
    pub outline_command_cache: HashMap<CacheKey, Option<Box<[swash::zeno::Command]>>>,
}

impl fmt::Debug for SwashCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("SwashCache { .. }")
    }
}

impl SwashCache {
    /// Create a new swash cache
    pub fn new() -> Self {
        Self {
            context: ScaleContext::new(),
            image_cache: HashMap::default(),
            outline_command_cache: HashMap::default(),
        }
    }

    /// Create a swash Image from a cache key, without caching results
    pub fn get_image_uncached(
        &mut self,
        font_system: &mut FontSystem,
        cache_key: CacheKey,
    ) -> Option<SwashImage> {
        swash_image(font_system, &mut self.context, cache_key)
    }

    /// Create a swash Image from a cache key, caching results
    pub fn get_image(
        &mut self,
        font_system: &mut FontSystem,
        cache_key: CacheKey,
    ) -> &Option<SwashImage> {
        self.image_cache
            .entry(cache_key)
            .or_insert_with(|| swash_image(font_system, &mut self.context, cache_key))
    }

    /// Creates outline commands
    pub fn get_outline_commands(
        &mut self,
        font_system: &mut FontSystem,
        cache_key: CacheKey,
    ) -> Option<&[swash::zeno::Command]> {
        self.outline_command_cache
            .entry(cache_key)
            .or_insert_with(|| swash_outline_commands(font_system, &mut self.context, cache_key))
            .as_deref()
    }

    /// Creates outline commands, without caching results
    pub fn get_outline_commands_uncached(
        &mut self,
        font_system: &mut FontSystem,
        cache_key: CacheKey,
    ) -> Option<Box<[swash::zeno::Command]>> {
        swash_outline_commands(font_system, &mut self.context, cache_key)
    }

    /// Enumerate pixels in an Image, use `with_image` for better performance
    pub fn with_pixels<F: FnMut(i32, i32, Color)>(
        &mut self,
        font_system: &mut FontSystem,
        cache_key: CacheKey,
        base: Color,
        mut f: F,
    ) {
        if let Some(image) = self.get_image(font_system, cache_key) {
            let x = image.placement.left;
            let y = -image.placement.top;

            match image.content {
                Content::Mask => {
                    let mut i = 0;
                    for off_y in 0..image.placement.height as i32 {
                        for off_x in 0..image.placement.width as i32 {
                            //TODO: blend base alpha?
                            f(
                                x + off_x,
                                y + off_y,
                                Color((u32::from(image.data[i]) << 24) | base.0 & 0xFF_FF_FF),
                            );
                            i += 1;
                        }
                    }
                }
                Content::Color => {
                    let mut i = 0;
                    for off_y in 0..image.placement.height as i32 {
                        for off_x in 0..image.placement.width as i32 {
                            //TODO: blend base alpha?
                            f(
                                x + off_x,
                                y + off_y,
                                Color::rgba(
                                    image.data[i],
                                    image.data[i + 1],
                                    image.data[i + 2],
                                    image.data[i + 3],
                                ),
                            );
                            i += 4;
                        }
                    }
                }
                Content::SubpixelMask => {
                    log::warn!("TODO: SubpixelMask");
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use swash::{FontRef, Setting, Tag};

    // variations() resizes context.coords in place (stale values persist),
    // whereas using normalized_coords() clears and replaces them.
    #[test]
    fn no_coord_leakage_across_fonts() {
        let [Ok(sfns), Ok(sfns_italic)] = [
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/SFNSItalic.ttf",
        ]
        .map(std::fs::read) else {
            return;
        };
        let regular = FontRef::from_index(&sfns, 0).unwrap();
        let italic = FontRef::from_index(&sfns_italic, 0).unwrap();
        let wght = Tag::from_be_bytes(*b"wght");

        let render = |ctx: &mut ScaleContext, font: FontRef, weight: f32, use_normalized| {
            let mut b = ctx.builder(font).size(16.0).hint(true);
            if use_normalized {
                b = b.normalized_coords(font.variations().normalized_coords([(wght, weight)]));
            } else {
                b = b.variations(std::iter::once(Setting {
                    tag: wght,
                    value: weight,
                }));
            }
            Render::new(&[Source::Outline])
                .format(Format::Alpha)
                .render(&mut b.build(), 36)
        };

        // reference: regular@400 with no prior context
        let mut ctx = ScaleContext::new();
        let reference = render(&mut ctx, regular, 400.0, false).map(|i| i.data);

        // variations(): pollute ctx with italic@700, then render regular@400
        let mut ctx = ScaleContext::new();
        render(&mut ctx, italic, 700.0, false);
        let not_normalized = render(&mut ctx, regular, 400.0, false).map(|i| i.data);

        // normalized_coords(): same sequence
        let mut ctx = ScaleContext::new();
        render(&mut ctx, italic, 700.0, true);
        let normalized = render(&mut ctx, regular, 400.0, true).map(|i| i.data);

        assert_ne!(not_normalized, reference, "variations leak across fonts");
        assert_eq!(
            normalized, reference,
            "normalized_coords match clean render"
        );
    }
}
