//! Contact sheets for `page.recordStart … contactSheet`: the frames that changed the page, in a
//! grid with timestamps and the changed regions outlined. Vendored from agent-browser
//! (`cli/src/native/recording.rs` at f0a9835, Apache-2.0, see NOTICE) without its cursor overlay.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// The default share of pixels that must change for a frame to join the sheet.
pub(crate) const DEFAULT_CONTACT_SHEET_THRESHOLD: f64 = 0.05;

/// Contact sheets stay reviewable and memory-bounded during long recordings.
pub(crate) const MAX_CONTACT_SHEET_FRAMES: usize = 100;

const CONTACT_SHEET_COLUMNS: u32 = 4;
const CONTACT_SHEET_CELL_WIDTH: u32 = 640;
const CONTACT_SHEET_LABEL_HEIGHT: u32 = 24;
const CONTACT_SHEET_GAP: u32 = 8;
const CONTACT_SHEET_DIFF_WIDTH: u32 = 320;
const CONTACT_SHEET_TILE_SIZE: u32 = 8;
const CONTACT_SHEET_MIN_TILE_PIXELS: u32 = 4;
const CONTACT_SHEET_MIN_REGION_PIXELS: u64 = 8;
const CONTACT_SHEET_REGION_PADDING_TILES: u32 = 1;
const CONTACT_SHEET_REGION_MERGE_GAP_TILES: u32 = 4;
const CONTACT_SHEET_BURST_WINDOW_MS: u64 = 1_000;
const CONTACT_SHEET_BURST_RATE: usize = 25;
const CONTACT_SHEET_BURST_QUIET_MS: u64 = 250;
const CONTACT_SHEET_BURST_FRAMES: usize = 7;

pub(crate) fn contact_sheet_path(output_path: &str) -> String {
    let path = Path::new(output_path);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("recording");
    let filename = format!("{}.contact-sheet.png", stem);
    path.parent()
        .unwrap_or_else(|| Path::new(""))
        .join(filename)
        .to_string_lossy()
        .to_string()
}

#[derive(Clone, Debug)]
struct ContactSheetFrame {
    source: Arc<image::RgbImage>,
    image_data: Vec<u8>,
    elapsed_ms: u64,
    change_ratio: f64,
}

pub(crate) struct ContactSheetCell {
    rendered: image::RgbaImage,
    elapsed_ms: u64,
}

struct ContactSheetBaseline {
    source: Arc<image::RgbImage>,
    preview: image::RgbImage,
}

pub(crate) struct ContactSheetCollector {
    threshold: f64,
    selected: Vec<ContactSheetCell>,
    pending: Vec<ContactSheetFrame>,
    burst: bool,
    previous_candidate: Option<ContactSheetBaseline>,
    previous_rendered: Option<Arc<image::RgbImage>>,
    latest: Option<ContactSheetFrame>,
    cell_height: Option<u32>,
}

impl ContactSheetCollector {
    pub(crate) fn new(threshold: f64) -> Self {
        Self {
            threshold,
            selected: Vec::new(),
            pending: Vec::new(),
            burst: false,
            previous_candidate: None,
            previous_rendered: None,
            latest: None,
            cell_height: None,
        }
    }

    fn commit_frame(&mut self, frame: ContactSheetFrame) {
        if self.selected.len() >= MAX_CONTACT_SHEET_FRAMES - 1 {
            return;
        }
        let cell = render_contact_cell(
            &frame,
            self.previous_rendered.as_deref(),
            self.cell_height.unwrap_or(1),
        );
        self.previous_rendered = Some(frame.source.clone());
        self.selected.push(cell);
    }

    fn flush_pending(&mut self) {
        let remaining = (MAX_CONTACT_SHEET_FRAMES - 1).saturating_sub(self.selected.len());
        let pending = std::mem::take(&mut self.pending);
        for frame in pending.into_iter().take(remaining) {
            self.commit_frame(frame);
        }
        self.burst = false;
    }

    fn flush_if_quiet(&mut self, elapsed_ms: u64) {
        if self.pending.last().is_some_and(|last| {
            elapsed_ms.saturating_sub(last.elapsed_ms) >= CONTACT_SHEET_BURST_QUIET_MS
        }) {
            self.flush_pending();
        }
    }

    fn compact_pending_burst(&mut self) {
        while self.pending.len() > CONTACT_SHEET_BURST_FRAMES {
            let remove = (1..self.pending.len() - 1)
                .min_by(|&a, &b| {
                    let score = |index: usize| {
                        let span = self.pending[index + 1]
                            .elapsed_ms
                            .saturating_sub(self.pending[index - 1].elapsed_ms)
                            as f64;
                        span * (1.0 + self.pending[index].change_ratio)
                    };
                    score(a)
                        .partial_cmp(&score(b))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("a burst with more than two frames has an interior frame");
            self.pending.remove(remove);
        }
    }

    fn stage_candidate(&mut self, frame: ContactSheetFrame) {
        self.flush_if_quiet(frame.elapsed_ms);
        self.pending.push(frame);

        if self.burst {
            self.compact_pending_burst();
            return;
        }

        let latest_ms = self.pending.last().map_or(0, |latest| latest.elapsed_ms);
        while self.pending.first().is_some_and(|first| {
            latest_ms.saturating_sub(first.elapsed_ms) > CONTACT_SHEET_BURST_WINDOW_MS
        }) {
            let frame = self.pending.remove(0);
            self.commit_frame(frame);
        }

        if self.pending.len() > CONTACT_SHEET_BURST_RATE {
            self.burst = true;
            self.compact_pending_burst();
        }
    }

    pub(crate) fn consider(&mut self, image_data: &[u8], elapsed: Duration) {
        let elapsed_ms = elapsed.as_millis().min(u64::MAX as u128) as u64;
        self.flush_if_quiet(elapsed_ms);
        // Encoded equality is only a shortcut; selection compares decoded pixels.
        if let Some(latest) = self
            .latest
            .as_mut()
            .filter(|frame| frame.image_data == image_data)
        {
            latest.elapsed_ms = elapsed_ms;
            return;
        }
        let Ok(source) = image::load_from_memory(image_data) else {
            return;
        };
        let preview_width = source.width().clamp(1, CONTACT_SHEET_DIFF_WIDTH);
        let preview_height = ((source.height() as f64 * preview_width as f64
            / source.width().max(1) as f64)
            .round() as u32)
            .max(1);
        // Integer averaging is sufficient for selection; preserve full pixels
        // for region detection and the high-quality cell rendering below.
        let preview = source
            .thumbnail_exact(preview_width, preview_height)
            .into_rgb8();
        let source = Arc::new(source.into_rgb8());
        self.cell_height.get_or_insert_with(|| {
            ((CONTACT_SHEET_CELL_WIDTH as f64 * source.height() as f64
                / source.width().max(1) as f64)
                .round() as u32)
                .max(1)
        });
        let frame = ContactSheetFrame {
            source,
            image_data: image_data.to_vec(),
            elapsed_ms,
            change_ratio: 0.0,
        };
        // Reserve one slot for the final frame. Compare with the last candidate
        // so small changes accumulate instead of disappearing.
        let change_ratio = self.previous_candidate.as_ref().map_or(1.0, |previous| {
            if previous.source.dimensions() != frame.source.dimensions() {
                1.0
            } else {
                changed_pixel_ratio(&previous.preview, &preview)
            }
        });
        if change_ratio > 0.0 && change_ratio >= self.threshold {
            let mut candidate = frame.clone();
            candidate.change_ratio = change_ratio;
            self.stage_candidate(candidate);
            self.previous_candidate = Some(ContactSheetBaseline {
                source: frame.source.clone(),
                preview,
            });
        }
        self.latest = Some(frame);
    }

    pub(crate) fn finish(mut self) -> Vec<ContactSheetCell> {
        self.flush_pending();
        if let Some(latest) = self.latest {
            let already_selected = self
                .selected
                .last()
                .is_some_and(|frame| frame.elapsed_ms == latest.elapsed_ms);
            if !already_selected {
                let cell = render_contact_cell(
                    &latest,
                    self.previous_rendered.as_deref(),
                    self.cell_height.unwrap_or(1),
                );
                self.selected.push(cell);
            }
        }
        self.selected
    }
}

fn changed_pixel_ratio(before: &image::RgbImage, after: &image::RgbImage) -> f64 {
    if before.dimensions() != after.dimensions() {
        return 1.0;
    }
    let (width, height) = after.dimensions();
    if width == 0 || height == 0 {
        return 0.0;
    }
    let changed = before
        .as_raw()
        .as_chunks::<3>()
        .0
        .iter()
        .zip(after.as_raw().as_chunks::<3>().0)
        .filter(|(a, b)| a != b)
        .count();
    changed as f64 / (width as u64 * height as u64) as f64
}

/// Normalized bounds of visually changed tile clusters. This runs only for
/// frames already selected as contact-sheet cells.
fn changed_pixel_regions(before: &image::RgbImage, after: &image::RgbImage) -> Vec<[f32; 4]> {
    if before.dimensions() != after.dimensions() {
        return vec![[0.0, 0.0, 1.0, 1.0]];
    }
    let (width, height) = after.dimensions();
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let tiles_wide = width.div_ceil(CONTACT_SHEET_TILE_SIZE);
    let tiles_high = height.div_ceil(CONTACT_SHEET_TILE_SIZE);
    let mut tile_counts = vec![0u32; (tiles_wide * tiles_high) as usize];
    for (index, (a, b)) in before
        .as_raw()
        .as_chunks::<3>()
        .0
        .iter()
        .zip(after.as_raw().as_chunks::<3>().0)
        .enumerate()
    {
        if a != b {
            let tile_x = (index % width as usize) as u32 / CONTACT_SHEET_TILE_SIZE;
            let tile_y = (index / width as usize) as u32 / CONTACT_SHEET_TILE_SIZE;
            tile_counts[(tile_y * tiles_wide + tile_x) as usize] += 1;
        }
    }
    let mut components: Vec<(u64, u32, u32, u32, u32)> = Vec::new();
    for start_y in 0..tiles_high {
        for start_x in 0..tiles_wide {
            let start = (start_y * tiles_wide + start_x) as usize;
            if tile_counts[start] < CONTACT_SHEET_MIN_TILE_PIXELS {
                continue;
            }
            let initial_pixels = tile_counts[start] as u64;
            tile_counts[start] = 0;
            let mut pending = std::collections::VecDeque::from([(start_x, start_y)]);
            let (mut min_x, mut min_y, mut max_x, mut max_y) = (start_x, start_y, start_x, start_y);
            let mut pixels = initial_pixels;
            while let Some((x, y)) = pending.pop_front() {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                for neighbor_y in y.saturating_sub(1)..=(y + 1).min(tiles_high - 1) {
                    for neighbor_x in x.saturating_sub(1)..=(x + 1).min(tiles_wide - 1) {
                        let neighbor = (neighbor_y * tiles_wide + neighbor_x) as usize;
                        if tile_counts[neighbor] >= CONTACT_SHEET_MIN_TILE_PIXELS {
                            pixels += tile_counts[neighbor] as u64;
                            tile_counts[neighbor] = 0;
                            pending.push_back((neighbor_x, neighbor_y));
                        }
                    }
                }
            }
            if pixels >= CONTACT_SHEET_MIN_REGION_PIXELS {
                components.push((pixels, min_x, min_y, max_x, max_y));
            }
        }
    }
    let gap = CONTACT_SHEET_REGION_MERGE_GAP_TILES;
    // Revisit all pairs after a union: the enlarged region may now reach a
    // component inspected earlier. Never discard regions to meet a box limit.
    let mut index = 0;
    while index < components.len() {
        let mut other = index + 1;
        while other < components.len() {
            let (_, ax1, ay1, ax2, ay2) = components[index];
            let (_, bx1, by1, bx2, by2) = components[other];
            let close = ax1 <= bx2.saturating_add(gap)
                && bx1 <= ax2.saturating_add(gap)
                && ay1 <= by2.saturating_add(gap)
                && by1 <= ay2.saturating_add(gap);
            if close {
                let merged = components.swap_remove(other);
                components[index].0 += merged.0;
                components[index].1 = components[index].1.min(merged.1);
                components[index].2 = components[index].2.min(merged.2);
                components[index].3 = components[index].3.max(merged.3);
                components[index].4 = components[index].4.max(merged.4);
                index = 0;
                other = 1;
            } else {
                other += 1;
            }
        }
        index += 1;
    }
    components.sort_by_key(|&(_, min_x, min_y, _, _)| (min_y, min_x));
    components
        .into_iter()
        .map(|(_, min_tile_x, min_tile_y, max_tile_x, max_tile_y)| {
            let min_x = min_tile_x.saturating_sub(CONTACT_SHEET_REGION_PADDING_TILES)
                * CONTACT_SHEET_TILE_SIZE;
            let min_y = min_tile_y.saturating_sub(CONTACT_SHEET_REGION_PADDING_TILES)
                * CONTACT_SHEET_TILE_SIZE;
            let max_x = ((max_tile_x + CONTACT_SHEET_REGION_PADDING_TILES + 1)
                * CONTACT_SHEET_TILE_SIZE)
                .min(width);
            let max_y = ((max_tile_y + CONTACT_SHEET_REGION_PADDING_TILES + 1)
                * CONTACT_SHEET_TILE_SIZE)
                .min(height);
            [
                min_x as f32 / width as f32,
                min_y as f32 / height as f32,
                (max_x - min_x) as f32 / width as f32,
                (max_y - min_y) as f32 / height as f32,
            ]
        })
        .collect()
}

fn format_contact_timestamp(milliseconds: u64) -> String {
    let hours = milliseconds / 3_600_000;
    let minutes = (milliseconds / 60_000) % 60;
    let seconds = (milliseconds / 1_000) % 60;
    let millis = milliseconds % 1_000;
    format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, seconds, millis)
}

fn glyph_rows(character: char) -> [u8; 7] {
    match character {
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        ':' => [0, 0b00100, 0b00100, 0, 0b00100, 0b00100, 0],
        '.' => [0, 0, 0, 0, 0, 0b00100, 0b00100],
        _ => [0; 7],
    }
}

fn draw_timestamp(image: &mut image::RgbaImage, x: u32, y: u32, value: &str) {
    const SCALE: u32 = 2;
    for (index, character) in value.chars().enumerate() {
        for (row, bits) in glyph_rows(character).iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) == 0 {
                    continue;
                }
                for dy in 0..SCALE {
                    for dx in 0..SCALE {
                        let px = x + index as u32 * 6 * SCALE + column * SCALE + dx;
                        let py = y + row as u32 * SCALE + dy;
                        if px < image.width() && py < image.height() {
                            image.put_pixel(px, py, image::Rgba([255, 255, 255, 255]));
                        }
                    }
                }
            }
        }
    }
}

fn draw_changed_region(image: &mut image::RgbaImage, x: u32, y: u32, width: u32, height: u32) {
    if width == 0 || height == 0 {
        return;
    }
    let right = (x + width - 1).min(image.width().saturating_sub(1));
    let bottom = (y + height - 1).min(image.height().saturating_sub(1));
    // A light tint keeps the content legible; the solid edge defines its bounds.
    for py in y..=bottom {
        for px in x..=right {
            let pixel = image.get_pixel_mut(px, py);
            for (channel, tint) in pixel.0[..3].iter_mut().zip([239u16, 68, 68]) {
                *channel = ((*channel as u16 * 7 + tint) / 8) as u8;
            }
        }
    }
    for thickness in 0..2 {
        let left = x.saturating_sub(thickness);
        let top = y.saturating_sub(thickness);
        let r = (right + thickness).min(image.width().saturating_sub(1));
        let b = (bottom + thickness).min(image.height().saturating_sub(1));
        for px in left..=r {
            image.put_pixel(px, top, image::Rgba([239, 68, 68, 255]));
            image.put_pixel(px, b, image::Rgba([239, 68, 68, 255]));
        }
        for py in top..=b {
            image.put_pixel(left, py, image::Rgba([239, 68, 68, 255]));
            image.put_pixel(r, py, image::Rgba([239, 68, 68, 255]));
        }
    }
}

fn render_contact_cell(
    frame: &ContactSheetFrame,
    previous: Option<&image::RgbImage>,
    cell_height: u32,
) -> ContactSheetCell {
    let source = frame.source.as_ref();
    let regions = previous
        .map(|before| changed_pixel_regions(before, source))
        .unwrap_or_default();
    let rendered = image::DynamicImage::ImageRgb8(source.clone())
        .resize(
            CONTACT_SHEET_CELL_WIDTH,
            cell_height,
            image::imageops::FilterType::Triangle,
        )
        .to_rgba8();
    let mut cell = image::RgbaImage::from_pixel(
        CONTACT_SHEET_CELL_WIDTH,
        cell_height + CONTACT_SHEET_LABEL_HEIGHT,
        image::Rgba([17, 24, 39, 255]),
    );
    let x = (CONTACT_SHEET_CELL_WIDTH - rendered.width()) / 2;
    copy_contact_cell(&mut cell, &rendered, x, CONTACT_SHEET_LABEL_HEIGHT);
    draw_timestamp(&mut cell, 4, 4, &format_contact_timestamp(frame.elapsed_ms));
    for [rx, ry, rw, rh] in regions {
        draw_changed_region(
            &mut cell,
            x + (rx * rendered.width() as f32).round() as u32,
            CONTACT_SHEET_LABEL_HEIGHT + (ry * rendered.height() as f32).round() as u32,
            (rw * rendered.width() as f32).round().max(1.0) as u32,
            (rh * rendered.height() as f32).round().max(1.0) as u32,
        );
    }
    ContactSheetCell {
        rendered: cell,
        elapsed_ms: frame.elapsed_ms,
    }
}

/// Assemble cells whose image analysis and rendering are already complete.
pub(crate) fn write_contact_sheet(path: &Path, frames: &[ContactSheetCell]) -> Result<(), String> {
    let first = frames
        .first()
        .map(|frame| &frame.rendered)
        .ok_or("No frames rendered for contact sheet")?;
    let columns = CONTACT_SHEET_COLUMNS.min(frames.len() as u32).max(1);
    let rows = (frames.len() as u32).div_ceil(columns);
    let width = CONTACT_SHEET_GAP + columns * (CONTACT_SHEET_CELL_WIDTH + CONTACT_SHEET_GAP);
    let height = CONTACT_SHEET_GAP + rows * (first.height() + CONTACT_SHEET_GAP);
    let mut canvas = image::RgbaImage::from_raw(
        width,
        height,
        [17, 24, 39, 255].repeat(width as usize * height as usize),
    )
    .expect("sheet dimensions");
    for (index, frame) in frames.iter().enumerate() {
        let cell = &frame.rendered;
        let x = CONTACT_SHEET_GAP
            + index as u32 % columns * (CONTACT_SHEET_CELL_WIDTH + CONTACT_SHEET_GAP);
        let y = CONTACT_SHEET_GAP + index as u32 / columns * (first.height() + CONTACT_SHEET_GAP);
        copy_contact_cell(&mut canvas, cell, x, y);
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create contact sheet directory: {}", e))?;
    }
    canvas
        .save(path)
        .map_err(|e| format!("Failed to save contact sheet: {}", e))
}

/// Contact-sheet images are opaque and fully inside the canvas; no blending is needed.
fn copy_contact_cell(canvas: &mut image::RgbaImage, cell: &image::RgbaImage, x: u32, y: u32) {
    let stride = canvas.width() as usize * 4;
    let row_bytes = cell.width() as usize * 4;
    for (row, pixels) in cell.as_raw().chunks_exact(row_bytes).enumerate() {
        let offset = (y as usize + row) * stride + x as usize * 4;
        canvas.as_mut()[offset..offset + row_bytes].copy_from_slice(pixels);
    }
}
