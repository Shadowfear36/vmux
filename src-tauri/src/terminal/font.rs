#![allow(dead_code)]
use cosmic_text::{
    Attrs, Buffer, Color as CosmicColor, Family, FontSystem, Metrics, Shaping, SwashCache,
    Weight, Style,
};

/// Manages font shaping and glyph rasterization via cosmic-text.
/// Handles ligatures, RTL, emoji, and CJK automatically.
pub struct FontManager {
    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
    pub cell_width: f32,
    pub cell_height: f32,
    pub font_size: f32,
}

impl FontManager {
    pub fn new(font_size: f32) -> Self {
        let mut font_system = FontSystem::new();
        let line_height = font_size * 1.2;
        let metrics = Metrics::new(font_size, line_height);

        // Measure cell width by shaping a reference monospace character
        let cell_width = measure_cell_width(&mut font_system, metrics, font_size);

        FontManager {
            font_system,
            swash_cache: SwashCache::new(),
            cell_width,
            cell_height: line_height,
            font_size,
        }
    }

    /// Rasterize a single terminal cell character.
    /// Returns RGBA pixel data (cell_width × cell_height × 4 bytes).
    pub fn rasterize_cell(
        &mut self,
        text: &str,
        bold: bool,
        italic: bool,
        fg: [u8; 4],
    ) -> Vec<u8> {
        let line_height = self.font_size * 1.2;
        let metrics = Metrics::new(self.font_size, line_height);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);

        let mut attrs = Attrs::new().family(Family::Monospace);
        if bold   { attrs = attrs.weight(Weight::BOLD); }
        if italic { attrs = attrs.style(Style::Italic); }

        let w = self.cell_width.ceil() as usize;
        let h = self.cell_height.ceil() as usize;

        // MUST set buffer size before shaping so cosmic-text positions the baseline
        // correctly within the cell height. Without this, run.line_y = 0 (baseline
        // at top) and glyph pixels get negative y coordinates, which silently wrap
        // to huge usize values → all pixels fail the bounds check → invisible text.
        buffer.set_size(&mut self.font_system, Some(w as f32), Some(h as f32));
        buffer.set_text(&mut self.font_system, text, attrs, Shaping::Advanced);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut pixels = vec![0u8; w * h * 4];

        let color = CosmicColor::rgba(fg[0], fg[1], fg[2], fg[3]);
        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            color,
            |px, py, _pw, _ph, c| {
                // px/py can be negative for some font metrics; skip those pixels.
                if px < 0 || py < 0 { return; }
                let x = px as usize;
                let y = py as usize;
                if x < w && y < h {
                    let idx = (y * w + x) * 4;
                    let a = c.a() as f32 / 255.0;
                    pixels[idx]     = (fg[0] as f32 * a) as u8;
                    pixels[idx + 1] = (fg[1] as f32 * a) as u8;
                    pixels[idx + 2] = (fg[2] as f32 * a) as u8;
                    pixels[idx + 3] = c.a();
                }
            },
        );

        pixels
    }
}

fn measure_cell_width(font_system: &mut FontSystem, metrics: Metrics, font_size: f32) -> f32 {
    let mut buf = Buffer::new(font_system, metrics);
    let attrs = Attrs::new().family(Family::Monospace);
    buf.set_text(font_system, "M", attrs, Shaping::Basic);
    buf.shape_until_scroll(font_system, false);

    for run in buf.layout_runs() {
        if !run.glyphs.is_empty() {
            return run.glyphs.iter().map(|g| g.w).sum::<f32>().max(1.0);
        }
    }
    font_size * 0.6
}
