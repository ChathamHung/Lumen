//! Background image decoding.
//!
//! Two worker threads keep the UI responsive:
//!   * one for the image currently being viewed (newest request wins, so
//!     flipping quickly through a folder never queues up work),
//!   * one for filmstrip thumbnails (small, cached by the caller).
//!
//! Decoding, EXIF rotation and downscaling all happen off the UI thread; only
//! the final texture upload touches the GPU.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;

/// Largest edge (in pixels) that is uploaded to the GPU. Larger images are
/// downscaled once, which keeps memory and upload time predictable.
pub const MAX_TEXTURE_EDGE: u32 = 8192;

/// Largest edge used for filmstrip thumbnails.
pub const THUMBNAIL_EDGE: u32 = 160;

/// Number of background threads that build filmstrip thumbnails.
const THUMBNAIL_WORKERS: usize = 3;

/// Nearest magnification keeps single pixels crisp when zoomed in (this is what
/// makes pixel-level inspection possible), while linear minification with
/// mipmaps keeps zoomed-out photos smooth and free of shimmer.
pub const IMAGE_TEXTURE_OPTIONS: egui::TextureOptions = egui::TextureOptions {
    magnification: egui::TextureFilter::Nearest,
    minification: egui::TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: Some(egui::TextureFilter::Linear),
};

/// One frame of an animated image.
pub struct AnimationFrame {
    /// Tightly packed RGBA pixels, `width * height * 4` bytes.
    pub pixels: Vec<u8>,
    /// How long this frame stays on screen, in milliseconds.
    pub delay_ms: u32,
}

/// A decoded image, still on the CPU.
pub struct DecodedImage {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub source_width: u32,
    pub source_height: u32,
    /// Pixels of the first frame, kept so single pixels can be inspected.
    pub pixels: Vec<u8>,
    /// Remaining frames of a GIF or APNG. Empty for still images.
    pub frames: Vec<AnimationFrame>,
    pub file_size: u64,
    pub modified: Option<SystemTime>,
    pub format: String,
    pub decode_ms: f32,
}

/// A decoded image that has been uploaded to the GPU.
pub struct LoadedImage {
    pub texture: egui::TextureHandle,
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub pixels: Vec<u8>,
    /// All frames when the file is animated; empty for a still image.
    pub frames: Vec<AnimationFrame>,
    /// Frame currently shown, and when the next one is due.
    pub frame_index: usize,
    pub frame_next: Instant,
    pub file_size: u64,
    pub modified: Option<SystemTime>,
    pub format: String,
    pub decode_ms: f32,
}

impl LoadedImage {
    /// Size in image pixels.
    pub fn size(&self) -> egui::Vec2 {
        egui::vec2(self.width as f32, self.height as f32)
    }

    /// Number of frames, 1 for a still image.
    pub fn frame_count(&self) -> usize {
        self.frames.len().max(1)
    }

    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// Pixels of the frame that is currently on screen.
    fn current_pixels(&self) -> &[u8] {
        match self.frames.get(self.frame_index) {
            Some(frame) if !self.frames.is_empty() => &frame.pixels,
            _ => &self.pixels,
        }
    }

    /// Straight (non-premultiplied) RGBA of a single pixel.
    pub fn pixel_rgba(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let pixels = self.current_pixels();
        let index = (y as usize * self.width as usize + x as usize) * 4;
        let slice = pixels.get(index..index + 4)?;
        Some([slice[0], slice[1], slice[2], slice[3]])
    }

    /// Moves to the next animation frame if one is due. Returns the time until
    /// the frame after that, so the caller can schedule a repaint.
    pub fn advance_frame(&mut self, now: Instant) -> Option<Duration> {
        if !self.is_animated() {
            return None;
        }
        if now < self.frame_next {
            return Some(self.frame_next - now);
        }

        self.frame_index = (self.frame_index + 1) % self.frames.len();
        let frame = &self.frames[self.frame_index];
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [self.width as usize, self.height as usize],
            &frame.pixels,
        );
        // `set` replaces the texture contents in place, so no new texture is
        // allocated while an animation runs.
        self.texture
            .set(egui::ImageData::Color(Arc::new(image)), IMAGE_TEXTURE_OPTIONS);

        let delay = Duration::from_millis(frame.delay_ms as u64);
        self.frame_next = now + delay;
        Some(delay)
    }
}

impl DecodedImage {
    /// Uploads the pixels and returns a ready-to-paint image.
    pub fn into_loaded(self, ctx: &egui::Context) -> LoadedImage {
        // The real limit of the GPU is only known once the backend has reported
        // it, and it can be lower than the 8192 asked for above — a software
        // rasteriser reports 2048. Scaling down here keeps a large photo from
        // panicking inside egui on such a machine.
        let limit = ctx.input(|input| input.max_texture_side).max(256) as u32;
        let (source_w, source_h) = (self.width, self.height);
        let mut pixels = self.pixels;
        let mut frames = self.frames;
        let (mut width, mut height) = (source_w, source_h);

        if source_w > limit || source_h > limit {
            let scale = limit as f32 / source_w.max(source_h) as f32;
            let target_w = ((source_w as f32 * scale).round() as u32).max(1);
            let target_h = ((source_h as f32 * scale).round() as u32).max(1);
            let shrink = |buffer: Vec<u8>| match image::RgbaImage::from_raw(source_w, source_h, buffer)
            {
                Some(image) => image::imageops::thumbnail(&image, target_w, target_h).into_raw(),
                None => Vec::new(),
            };
            pixels = shrink(pixels);
            for frame in &mut frames {
                frame.pixels = shrink(std::mem::take(&mut frame.pixels));
            }
            width = target_w;
            height = target_h;
            crate::diag::log(&format!(
                "texture limit {limit} px: scaled {source_w}x{source_h} down to {width}x{height}"
            ));
        }

        let color_image =
            egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &pixels);
        let name = format!("lumen.image:{}", self.path.display());
        let texture = ctx.load_texture(name, color_image, IMAGE_TEXTURE_OPTIONS);

        let frame_next = Instant::now()
            + Duration::from_millis(frames.first().map_or(100, |frame| frame.delay_ms) as u64);

        LoadedImage {
            texture,
            path: self.path,
            width,
            height,
            source_width: self.source_width,
            source_height: self.source_height,
            pixels,
            frames,
            frame_index: 0,
            frame_next,
            file_size: self.file_size,
            modified: self.modified,
            format: self.format,
            decode_ms: self.decode_ms,
        }
    }
}

struct Request {
    path: PathBuf,
    generation: u64,
    max_edge: u32,
}

/// Answer from the decoding thread.
pub struct Response {
    pub generation: u64,
    pub result: Result<DecodedImage, String>,
}

/// Loader for the image that is currently on screen.
pub struct ImageLoader {
    tx: Sender<Request>,
    rx: Receiver<Response>,
    generation: u64,
}

impl ImageLoader {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, requests) = channel::<Request>();
        let (responses, rx) = channel::<Response>();

        std::thread::Builder::new()
            .name("lumen-decode".to_owned())
            .spawn(move || {
                while let Ok(mut request) = requests.recv() {
                    // Coalesce queued work: only the newest request matters.
                    while let Ok(newer) = requests.try_recv() {
                        request = newer;
                    }
                    let result = decode(&request.path, request.max_edge);
                    if responses
                        .send(Response {
                            generation: request.generation,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .ok();

        Self {
            tx,
            rx,
            generation: 0,
        }
    }

    /// Queues a file and returns the generation number of this request.
    pub fn request(&mut self, path: PathBuf, max_edge: u32) -> u64 {
        self.generation += 1;
        let _ = self.tx.send(Request {
            path,
            generation: self.generation,
            max_edge,
        });
        self.generation
    }

    pub fn poll(&self) -> Option<Response> {
        self.rx.try_recv().ok()
    }
}

/// A decoded filmstrip thumbnail.
pub struct Thumbnail {
    pub path: PathBuf,
    pub size: [usize; 2],
    pub pixels: Vec<u8>,
}

pub struct ThumbnailResult {
    pub path: PathBuf,
    pub result: Result<Thumbnail, String>,
}

/// Loader for filmstrip thumbnails.
pub struct ThumbnailLoader {
    tx: Sender<PathBuf>,
    rx: Receiver<ThumbnailResult>,
    in_flight: HashSet<PathBuf>,
}

impl ThumbnailLoader {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, requests) = channel::<PathBuf>();
        let (responses, rx) = channel::<ThumbnailResult>();
        let requests = Arc::new(Mutex::new(requests));

        // A small pool, because decoding a 24 megapixel photo just to shrink it
        // to 160 pixels takes a few hundred milliseconds. One worker per core
        // would waste memory on big photos, so keep it modest.
        for index in 0..THUMBNAIL_WORKERS {
            let requests = Arc::clone(&requests);
            let responses = responses.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name(format!("lumen-thumbs-{index}"))
                .spawn(move || loop {
                    // The lock is only held while waiting for the next request,
                    // never while decoding.
                    let Ok(path) = requests.lock().map(|queue| queue.recv()) else {
                        break;
                    };
                    let Ok(path) = path else { break };

                    let result = decode(&path, THUMBNAIL_EDGE).map(|decoded| Thumbnail {
                        path: decoded.path.clone(),
                        size: [decoded.width as usize, decoded.height as usize],
                        pixels: decoded.pixels,
                    });
                    if responses.send(ThumbnailResult { path, result }).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                })
                .ok();
        }

        Self {
            tx,
            rx,
            in_flight: HashSet::new(),
        }
    }

    /// Queues a thumbnail unless it is already being decoded.
    pub fn request(&mut self, path: PathBuf) {
        // Do not let a fast scroll build up an unbounded queue: past this many
        // pending decodes the visible thumbnails would wait behind stale ones.
        if self.in_flight.len() > 48 || !self.in_flight.insert(path.clone()) {
            return;
        }
        let _ = self.tx.send(path);
    }

    pub fn poll(&mut self) -> Option<ThumbnailResult> {
        let result = self.rx.try_recv().ok()?;
        self.in_flight.remove(&result.path);
        Some(result)
    }
}

/// Name shown in the info overlay for a decoded format.
///
/// Most formats are named after their first extension, but PNM is a family of
/// four file types (PBM, PGM, PPM and PAM) that share one decoder, so using its
/// first extension would label every portable map "PBM". The umbrella name is
/// the honest one.
fn format_label(format: image::ImageFormat) -> &'static str {
    match format {
        image::ImageFormat::Pnm => "PNM",
        other => other.extensions_str().first().copied().unwrap_or("?"),
    }
}

// ---------------------------------------------------------------------------
// SVG and PSD, which the `image` crate cannot read
// ---------------------------------------------------------------------------

/// Largest raster an SVG is turned into. Vector art stays sharp at any zoom,
/// but the texture has to be finite: 2048 px is already 16 MB of pixels, and
/// far more than a screen can show at once.
const SVG_MAX_EDGE: u32 = 2048;

/// Upper bound on the upscaling of a small SVG. A 16x16 icon would otherwise
/// be blown up to 2048 px, which costs memory and buys nothing.
const SVG_MAX_SCALE: f32 = 48.0;

/// A picture Lumen decoded itself, in straight RGBA at its final size.
struct SelfDecoded {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    /// Size reported in the info overlay, which for a vector image is the size
    /// declared in the file rather than the raster it was rendered at.
    source_width: u32,
    source_height: u32,
    format: &'static str,
}

/// True when the file looks like SVG.
///
/// SVG has no binary signature, so the root element is searched for instead.
/// The XML declaration, a doctype and comments may all come first, which is why
/// the search covers the whole head of the file rather than a fixed offset.
fn looks_like_svg(bytes: &[u8]) -> bool {
    bytes
        .windows(4)
        .take(4096)
        .any(|window| window.eq_ignore_ascii_case(b"<svg"))
}

/// True when the SVG contains text, which is the only case where the (slow)
/// system font scan is worth paying for.
fn contains_text(bytes: &[u8]) -> bool {
    bytes
        .windows(5)
        .any(|window| window.eq_ignore_ascii_case(b"<text"))
}

/// Font database for SVG text, filled on first use only.
fn system_fonts() -> std::sync::Arc<resvg::usvg::fontdb::Database> {
    static FONTS: std::sync::OnceLock<std::sync::Arc<resvg::usvg::fontdb::Database>> =
        std::sync::OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut database = resvg::usvg::fontdb::Database::new();
            database.load_system_fonts();
            std::sync::Arc::new(database)
        })
        .clone()
}

/// Rasterises an SVG at the resolution the current view needs.
fn decode_svg(bytes: &[u8], max_edge: u32) -> Result<SelfDecoded, String> {
    // The default family is "Times New Roman", which is not guaranteed to
    // exist; the UI font is a better guess for the rare SVG that has text.
    let mut options = resvg::usvg::Options {
        font_family: "Segoe UI".to_owned(),
        ..Default::default()
    };
    if contains_text(bytes) {
        options.fontdb = system_fonts();
    }

    let tree =
        resvg::usvg::Tree::from_data(bytes, &options).map_err(|error| format!("svg: {error}"))?;
    let size = tree.size();
    let (width, height) = (size.width(), size.height());
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("svg: the drawing has no usable size".to_owned());
    }

    let target = max_edge.clamp(1, SVG_MAX_EDGE) as f32;
    // Rendering a 480 px drawing at 2048 px costs sixteen times the memory of
    // rendering it at 960 and looks the same on screen. Twice the declared size
    // keeps vector art sharp when zoomed in, with a floor so small icons still
    // have pixels to show.
    let intrinsic = width.max(height);
    let wanted = (intrinsic * 2.0).clamp(256.0, SVG_MAX_EDGE as f32);
    let scale = (wanted.min(target) / intrinsic).clamp(f32::MIN_POSITIVE, SVG_MAX_SCALE);
    let pixel_width = ((width * scale).round() as u32).max(1);
    let pixel_height = ((height * scale).round() as u32).max(1);

    let mut pixmap = resvg::tiny_skia::Pixmap::new(pixel_width, pixel_height)
        .ok_or_else(|| "svg: could not allocate the render target".to_owned())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // tiny-skia works with premultiplied alpha, egui and the rest of this file
    // with straight alpha, so the colour is divided back out here.
    let mut pixels = Vec::with_capacity((pixel_width * pixel_height * 4) as usize);
    for pixel in pixmap.pixels() {
        let colour = pixel.demultiply();
        pixels.extend_from_slice(&[
            colour.red(),
            colour.green(),
            colour.blue(),
            colour.alpha(),
        ]);
    }

    Ok(SelfDecoded {
        pixels,
        width: pixel_width,
        height: pixel_height,
        source_width: width.round().max(1.0) as u32,
        source_height: height.round().max(1.0) as u32,
        format: "SVG",
    })
}

/// Reads the flattened picture out of a PSD file.
fn decode_psd(bytes: &[u8], max_edge: u32) -> Result<SelfDecoded, String> {
    let decoded = crate::psd::decode(bytes)?;
    let (source_width, source_height) = (decoded.width, decoded.height);
    let image = image::RgbaImage::from_raw(decoded.width, decoded.height, decoded.pixels)
        .ok_or_else(|| "psd: the decoded buffer has the wrong size".to_owned())?;

    // A PSD is often a print-sized document, so the texture is capped the same
    // way photographs are.
    let image = if source_width.max(source_height) > max_edge {
        let scale = max_edge as f32 / source_width.max(source_height) as f32;
        let target_width = ((source_width as f32 * scale).round() as u32).max(1);
        let target_height = ((source_height as f32 * scale).round() as u32).max(1);
        image::imageops::thumbnail(&image, target_width, target_height)
    } else {
        image
    };

    let (width, height) = image.dimensions();
    Ok(SelfDecoded {
        pixels: image.into_raw(),
        width,
        height,
        source_width,
        source_height,
        format: "PSD",
    })
}

/// Reads, decodes, rotates and (if necessary) downscales one file.
fn decode(path: &Path, max_edge: u32) -> Result<DecodedImage, String> {
    let started = Instant::now();
    let bytes = std::fs::read(path).map_err(|error| format!("read failed: {error}"))?;
    let file_size = bytes.len() as u64;
    let modified = std::fs::metadata(path).ok().and_then(|meta| meta.modified().ok());

    // SVG and PSD are decoded by Lumen itself; neither has a decoder in the
    // `image` crate, and both are finished before the generic path starts. The
    // extension is tried first because a gzipped SVG (`.svgz`) has no readable
    // content to sniff, then the content decides for files with other names.
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let special = match extension.as_str() {
        "svg" | "svgz" => Some(decode_svg(&bytes, max_edge)?),
        "psd" => Some(decode_psd(&bytes, max_edge)?),
        _ if looks_like_svg(&bytes) => Some(decode_svg(&bytes, max_edge)?),
        _ if bytes.starts_with(b"8BPS") => Some(decode_psd(&bytes, max_edge)?),
        _ => None,
    };
    if let Some(decoded) = special {
        return Ok(DecodedImage {
            path: path.to_path_buf(),
            width: decoded.width,
            height: decoded.height,
            source_width: decoded.source_width,
            source_height: decoded.source_height,
            pixels: decoded.pixels,
            frames: Vec::new(),
            file_size,
            modified,
            format: decoded.format.to_owned(),
            decode_ms: started.elapsed().as_secs_f32() * 1000.0,
        });
    }

    let guessed = image::guess_format(&bytes).ok();
    let mut format = guessed.map(format_label).unwrap_or("?").to_ascii_uppercase();

    // Animated files get their own frame loop. Anything that goes wrong there
    // falls through to the ordinary single-image decoder below, so a broken
    // animation still shows its first frame instead of an error.
    let animated = match guessed {
        Some(image::ImageFormat::Gif) => decode_gif(&bytes, max_edge).ok(),
        Some(image::ImageFormat::Png) if has_apng_chunk(&bytes) => {
            let decoded = decode_apng(&bytes, max_edge).ok();
            if decoded.is_some() {
                format = "APNG".to_owned();
            }
            decoded
        }
        _ => None,
    };

    if let Some(animated) = animated {
        return Ok(DecodedImage {
            path: path.to_path_buf(),
            width: animated.width,
            height: animated.height,
            source_width: animated.source_width,
            source_height: animated.source_height,
            pixels: animated.pixels,
            frames: animated.frames,
            file_size,
            modified,
            format,
            decode_ms: started.elapsed().as_secs_f32() * 1000.0,
        });
    }

    let orientation = exif_orientation(&bytes);

    let sniffed = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|error| format!("format detection failed: {error}"))?;

    // A few formats have no magic bytes in their header, TGA being the common
    // one, so sniffing the contents cannot identify them. For those files the
    // extension is the only hint left, and using it beats refusing to open a
    // file that the folder listing already accepted.
    let reader = match (sniffed.format(), path.extension().and_then(image::ImageFormat::from_extension)) {
        (None, Some(hinted)) => {
            format = format_label(hinted).to_ascii_uppercase();
            image::ImageReader::with_format(std::io::Cursor::new(&bytes), hinted)
        }
        _ => sniffed,
    };

    let decoded = reader.decode().map_err(|error| format!("decode failed: {error}"))?;
    let (source_width, source_height) = (decoded.width(), decoded.height());

    // Shrink first and convert to RGBA afterwards. A 24 megapixel photo only
    // needs a 160 pixel thumbnail, and converting it to RGBA at full size would
    // waste roughly 40 MB per photo.
    let scaled = if source_width.max(source_height) > max_edge {
        let scale = max_edge as f32 / source_width.max(source_height) as f32;
        let target_w = ((source_width as f32 * scale).round() as u32).max(1);
        let target_h = ((source_height as f32 * scale).round() as u32).max(1);
        decoded.thumbnail(target_w, target_h)
    } else {
        decoded
    };

    let rgba = apply_orientation(scaled.to_rgba8(), orientation);
    let (width, height) = (rgba.width(), rgba.height());

    Ok(DecodedImage {
        path: path.to_path_buf(),
        width,
        height,
        source_width,
        source_height,
        pixels: rgba.into_raw(),
        frames: Vec::new(),
        file_size,
        modified,
        format,
        decode_ms: started.elapsed().as_secs_f32() * 1000.0,
    })
}

// ---------------------------------------------------------------------------
// Animation
// ---------------------------------------------------------------------------

/// Upper bound on the frames kept for one animation, and on their total size.
/// A long screen recording converted to GIF can otherwise eat gigabytes.
const MAX_ANIMATION_FRAMES: usize = 400;
const ANIMATION_BUDGET_BYTES: usize = 192 * 1024 * 1024;

/// A decoded animation: the first frame plus every following frame.
struct AnimatedImage {
    width: u32,
    height: u32,
    source_width: u32,
    source_height: u32,
    pixels: Vec<u8>,
    frames: Vec<AnimationFrame>,
}

/// Turns a raw decoder delay into something usable. Browsers treat a missing or
/// sub-20 ms delay as 100 ms, and so do we, otherwise frames flicker past.
fn normalize_delay(delay_ms: u32) -> u32 {
    if delay_ms <= 20 {
        100
    } else {
        delay_ms.min(10_000)
    }
}

/// Shrinks one frame if it is larger than the texture limit allows.
fn fit_frame(pixels: Vec<u8>, width: u32, height: u32, max_edge: u32) -> (Vec<u8>, u32, u32) {
    if width.max(height) <= max_edge || width == 0 || height == 0 {
        return (pixels, width, height);
    }
    let Some(buffer) = image::RgbaImage::from_raw(width, height, pixels) else {
        return (Vec::new(), width, height);
    };
    let scale = max_edge as f32 / width.max(height) as f32;
    let target_w = ((width as f32 * scale).round() as u32).max(1);
    let target_h = ((height as f32 * scale).round() as u32).max(1);
    let small = image::imageops::thumbnail(&buffer, target_w, target_h);
    (small.into_raw(), target_w, target_h)
}

/// Decodes every frame of a GIF. The `image` crate composites the frames and
/// applies the disposal methods for us, so each frame is a full canvas.
fn decode_gif(bytes: &[u8], max_edge: u32) -> Result<AnimatedImage, String> {
    use image::AnimationDecoder;

    let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes))
        .map_err(|error| error.to_string())?;

    let mut frames: Vec<AnimationFrame> = Vec::new();
    let mut budget = ANIMATION_BUDGET_BYTES;
    let mut canvas = (0u32, 0u32);
    let mut source = (0u32, 0u32);

    for frame in decoder.into_frames() {
        let frame = frame.map_err(|error| error.to_string())?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        // A GIF frame delay of 0/0 is legal and means "as fast as possible".
        let delay_ms = normalize_delay(numerator.checked_div(denominator).unwrap_or(0));

        let buffer = frame.into_buffer();
        let (width, height) = (buffer.width(), buffer.height());
        if frames.is_empty() {
            source = (width, height);
        }
        let (pixels, width, height) = fit_frame(buffer.into_raw(), width, height, max_edge);
        if frames.is_empty() {
            canvas = (width, height);
        }

        // Stop before the animation eats all available memory.
        if frames.len() >= MAX_ANIMATION_FRAMES || pixels.len() > budget {
            break;
        }
        budget -= pixels.len();
        frames.push(AnimationFrame { pixels, delay_ms });
    }

    finish_animation(frames, canvas, source)
}

/// Decodes an APNG by compositing each frame onto a canvas, as the specification
/// describes. The `image` crate cannot do this, hence the manual work.
fn decode_apng(bytes: &[u8], max_edge: u32) -> Result<AnimatedImage, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // Expands palettes and transparency so every frame arrives as 8-bit RGBA.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;

    let canvas_width = reader.info().width;
    let canvas_height = reader.info().height;
    if canvas_width == 0 || canvas_height == 0 {
        return Err("empty animation".to_owned());
    }

    let mut canvas = vec![0u8; canvas_width as usize * canvas_height as usize * 4];
    let mut buffer = vec![0u8; canvas_width as usize * canvas_height as usize * 4];
    let mut frames: Vec<AnimationFrame> = Vec::new();
    let mut budget = ANIMATION_BUDGET_BYTES;
    // Frames are only resized when the canvas is larger than the texture limit,
    // in which case the animation dimensions follow the resized frames.
    let mut animation_size = (canvas_width, canvas_height);

    // Running out of frames is the normal way out of this loop, so the error is
    // not an error: every animation simply ends with one.
    while let Ok(info) = reader.next_frame(&mut buffer) {
        let control = reader.info().frame_control;

        // Snapshot the canvas so a "restore to previous" frame can undo itself.
        let restore = match control.as_ref().map(|c| c.dispose_op) {
            Some(png::DisposeOp::Previous) => Some(canvas.clone()),
            _ => None,
        };

        let frame = frame_to_rgba(&buffer, &info);
        let (offset_x, offset_y) = control
            .as_ref()
            .map_or((0, 0), |c| (c.x_offset, c.y_offset));
        let blend_over = control
            .as_ref()
            .map(|c| c.blend_op == png::BlendOp::Over)
            .unwrap_or(false);
        composite(
            &mut canvas,
            canvas_width,
            canvas_height,
            &frame,
            info.width,
            info.height,
            offset_x,
            offset_y,
            blend_over,
        );

        let delay_ms = normalize_delay(match control.as_ref() {
            Some(control) => {
                let denominator = if control.delay_den == 0 {
                    100
                } else {
                    control.delay_den as u32
                };
                (control.delay_num as u32 * 1000) / denominator
            }
            None => 0,
        });

        let (pixels, width, height) = fit_frame(canvas.clone(), canvas_width, canvas_height, max_edge);
        if frames.is_empty() {
            animation_size = (width, height);
        }
        if frames.len() >= MAX_ANIMATION_FRAMES || pixels.len() > budget {
            break;
        }
        budget -= pixels.len();
        frames.push(AnimationFrame { pixels, delay_ms });

        // Dispose before the next frame is drawn.
        match control.as_ref().map(|c| c.dispose_op) {
            Some(png::DisposeOp::Background) => {
                clear_rect(
                    &mut canvas,
                    canvas_width,
                    canvas_height,
                    info.width,
                    info.height,
                    offset_x,
                    offset_y,
                );
            }
            Some(png::DisposeOp::Previous) => {
                if let Some(previous) = restore {
                    canvas = previous;
                }
            }
            _ => {}
        }
    }

    finish_animation(frames, animation_size, (canvas_width, canvas_height))
}

/// Converts one decoded PNG frame into tightly packed RGBA.
fn frame_to_rgba(buffer: &[u8], info: &png::OutputInfo) -> Vec<u8> {
    let (width, height) = (info.width as usize, info.height as usize);
    let mut rgba = vec![0u8; width * height * 4];
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        _ => 1,
    };

    for y in 0..height {
        let row = &buffer[y * info.line_size..];
        for x in 0..width {
            let source = &row[x * channels..x * channels + channels];
            let target = &mut rgba[(y * width + x) * 4..][..4];
            match channels {
                4 => target.copy_from_slice(source),
                3 => {
                    target[0] = source[0];
                    target[1] = source[1];
                    target[2] = source[2];
                    target[3] = 255;
                }
                2 => {
                    target[0] = source[0];
                    target[1] = source[0];
                    target[2] = source[0];
                    target[3] = source[1];
                }
                _ => {
                    target[0] = source[0];
                    target[1] = source[0];
                    target[2] = source[0];
                    target[3] = 255;
                }
            }
        }
    }
    rgba
}

/// Draws one frame onto the canvas, replacing or alpha-blending as asked.
#[allow(clippy::too_many_arguments)]
fn composite(
    canvas: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    frame: &[u8],
    width: u32,
    height: u32,
    offset_x: u32,
    offset_y: u32,
    blend_over: bool,
) {
    for y in 0..height {
        let canvas_y = offset_y + y;
        if canvas_y >= canvas_height {
            break;
        }
        for x in 0..width {
            let canvas_x = offset_x + x;
            if canvas_x >= canvas_width {
                break;
            }
            let source = &frame[(y as usize * width as usize + x as usize) * 4..][..4];
            let index = (canvas_y as usize * canvas_width as usize + canvas_x as usize) * 4;
            let target = &mut canvas[index..index + 4];
            if blend_over {
                let alpha = source[3] as u32;
                if alpha == 0 {
                    continue;
                }
                if alpha == 255 {
                    target.copy_from_slice(source);
                } else {
                    // Standard source-over compositing on straight alpha.
                    let inverse = 255 - alpha;
                    for channel in 0..3 {
                        target[channel] = ((source[channel] as u32 * alpha
                            + target[channel] as u32 * inverse)
                            / 255) as u8;
                    }
                    target[3] = (alpha + target[3] as u32 * inverse / 255).min(255) as u8;
                }
            } else {
                target.copy_from_slice(source);
            }
        }
    }
}

/// Clears a rectangle of the canvas to transparent.
fn clear_rect(
    canvas: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    width: u32,
    height: u32,
    offset_x: u32,
    offset_y: u32,
) {
    for y in 0..height {
        let canvas_y = offset_y + y;
        if canvas_y >= canvas_height {
            break;
        }
        let start = (canvas_y as usize * canvas_width as usize + offset_x.min(canvas_width) as usize) * 4;
        let end = ((canvas_y as usize * canvas_width as usize)
            + (offset_x + width).min(canvas_width) as usize)
            * 4;
        if end > start {
            canvas[start..end].fill(0);
        }
    }
}

/// Common tail of both animation decoders: validate the frame list.
fn finish_animation(
    frames: Vec<AnimationFrame>,
    (width, height): (u32, u32),
    (source_width, source_height): (u32, u32),
) -> Result<AnimatedImage, String> {
    if frames.is_empty() {
        return Err("no frames".to_owned());
    }
    // A single frame is just a still image; no need to animate anything.
    let pixels = frames[0].pixels.clone();
    let frames = if frames.len() > 1 { frames } else { Vec::new() };
    Ok(AnimatedImage {
        width,
        height,
        source_width,
        source_height,
        pixels,
        frames,
    })
}

/// True when a PNG contains an `acTL` chunk before the image data, which is
/// what makes it an animated PNG.
fn has_apng_chunk(bytes: &[u8]) -> bool {
    if bytes.len() < 8 || &bytes[1..4] != b"PNG" {
        return false;
    }
    let mut offset = 8;
    while offset + 8 <= bytes.len() {
        let length = u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        let kind = &bytes[offset + 4..offset + 8];
        if kind == b"acTL" {
            return true;
        }
        if kind == b"IDAT" {
            return false;
        }
        match offset.checked_add(12).and_then(|next| next.checked_add(length)) {
            Some(next) => offset = next,
            None => return false,
        }
    }
    false
}

/// Reads the EXIF orientation tag (1..=8). Returns 1 when there is none.
fn exif_orientation(bytes: &[u8]) -> u32 {
    let mut cursor = std::io::Cursor::new(bytes);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut cursor) else {
        return 1;
    };
    exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|field| field.value.get_uint(0))
        .unwrap_or(1)
}

/// Applies the EXIF orientation so phone photos are not shown sideways.
fn apply_orientation(image: image::RgbaImage, orientation: u32) -> image::RgbaImage {
    use image::imageops;
    match orientation {
        2 => imageops::flip_horizontal(&image),
        3 => imageops::rotate180(&image),
        4 => imageops::flip_vertical(&image),
        5 => imageops::rotate90(&imageops::flip_horizontal(&image)),
        6 => imageops::rotate90(&image),
        7 => imageops::rotate270(&imageops::flip_horizontal(&image)),
        8 => imageops::rotate270(&image),
        _ => image,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_large_jpeg_thumbnail() {
        for name in ["wallpaper-1.jpg", "风景照片.jpg"] {
            let path = Path::new("test-images").join(name);
            match decode(&path, THUMBNAIL_EDGE) {
                Ok(image) => println!(
                    "{name}: {}x{} px={} in {:.1} ms",
                    image.width,
                    image.height,
                    image.pixels.len() / 4,
                    image.decode_ms
                ),
                Err(error) => panic!("{name}: {error}"),
            }
        }
    }

    /// Writes one small file per supported format and reads it back through the
    /// real decoding path. This is what proves the claim that Lumen opens "more
    /// than JPEG and PNG": every format the build enables has to survive a
    /// round trip, including the formats that have no dedicated code above.
    #[test]
    fn decodes_every_supported_still_format() {
        // Every format the build enables that has an encoder.
        let formats = [
            (image::ImageFormat::Bmp, "bmp"),
            (image::ImageFormat::Tga, "tga"),
            (image::ImageFormat::Tiff, "tif"),
            (image::ImageFormat::Qoi, "qoi"),
            (image::ImageFormat::Pnm, "ppm"),
            (image::ImageFormat::Farbfeld, "ff"),
            (image::ImageFormat::Ico, "ico"),
            (image::ImageFormat::WebP, "webp"),
            (image::ImageFormat::Hdr, "hdr"),
            (image::ImageFormat::OpenExr, "exr"),
            (image::ImageFormat::Png, "png"),
            (image::ImageFormat::Jpeg, "jpg"),
            (image::ImageFormat::Gif, "gif"),
        ];

        let dir = std::env::temp_dir().join("lumen-format-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp folder");

        for (format, extension) in formats {
            // A 5x4 image with a known colour in the top-left corner.
            let mut source = image::RgbaImage::new(5, 4);
            for (x, y, pixel) in source.enumerate_pixels_mut() {
                *pixel = image::Rgba([200, (x * 10) as u8, (y * 20) as u8, 255]);
            }
            // Three formats accept neither 8-bit nor RGBA samples, and
            // `write_to` does not convert for them, so those fixtures are
            // created at the depth and channel count their encoder demands.
            let source = image::DynamicImage::ImageRgba8(source);
            let source = match format {
                image::ImageFormat::Farbfeld => image::DynamicImage::ImageRgba16(source.to_rgba16()),
                image::ImageFormat::Hdr => image::DynamicImage::ImageRgb32F(source.to_rgb32f()),
                image::ImageFormat::OpenExr => image::DynamicImage::ImageRgba32F(source.to_rgba32f()),
                _ => source,
            };
            let mut buffer = std::io::Cursor::new(Vec::new());
            source
                .write_to(&mut buffer, format)
                .unwrap_or_else(|error| panic!("could not write a {extension} fixture: {error}"));

            let path = dir.join(format!("sample.{extension}"));
            std::fs::write(&path, buffer.into_inner()).expect("write fixture");

            let decoded =
                decode(&path, MAX_TEXTURE_EDGE).unwrap_or_else(|error| panic!("{extension}: {error}"));
            assert_eq!(
                (decoded.width, decoded.height),
                (5, 4),
                "{extension} decoded with the wrong size"
            );
            assert!(
                decoded.frames.is_empty(),
                "{extension} must not be treated as an animation"
            );
            println!(
                "{extension:>4}: format {:>5}, {}x{}, {} bytes",
                decoded.format,
                decoded.width,
                decoded.height,
                decoded.file_size
            );
        }

        // DDS is the one format with no encoder in the `image` crate, so the
        // file is built by hand: a 4x4 DXT1 texture that is entirely red.
        let path = dir.join("sample.dds");
        std::fs::write(&path, dxt1_red_texture(4, 4)).expect("write dds fixture");
        let decoded = decode(&path, MAX_TEXTURE_EDGE).expect("dds should decode");
        assert_eq!((decoded.width, decoded.height), (4, 4));
        assert_eq!(decoded.format, "DDS");
        let [red, green, blue, _] = decoded.pixels[0..4] else {
            panic!("the dds fixture should decode to at least one pixel");
        };
        assert!(
            red > 200 && green < 40 && blue < 40,
            "the DXT1 block should decode to red, got ({red}, {green}, {blue})"
        );
        println!(
            " dds: format {:>5}, {}x{}, {} bytes",
            decoded.format,
            decoded.width,
            decoded.height,
            decoded.file_size
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Builds a minimal DXT1-compressed DDS file filled with solid red.
    ///
    /// The header layout is documented at
    /// <https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dds-header>.
    fn dxt1_red_texture(width: u32, height: u32) -> Vec<u8> {
        const HEADER_SIZE: u32 = 124;
        // CAPS | HEIGHT | WIDTH | PIXELFORMAT | LINEARSIZE
        const FLAGS: u32 = 0x1 | 0x2 | 0x4 | 0x1000 | 0x80000;
        const PIXEL_FORMAT_SIZE: u32 = 32;
        const DDPF_FOURCC: u32 = 0x4;
        const DDSCAPS_TEXTURE: u32 = 0x1000;

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"DDS ");
        bytes.extend_from_slice(&HEADER_SIZE.to_le_bytes());
        bytes.extend_from_slice(&FLAGS.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&8u32.to_le_bytes()); // one 8-byte block
        bytes.extend_from_slice(&0u32.to_le_bytes()); // depth
        bytes.extend_from_slice(&0u32.to_le_bytes()); // mipmap count
        bytes.extend_from_slice(&[0u8; 44]); // dwReserved1
        bytes.extend_from_slice(&PIXEL_FORMAT_SIZE.to_le_bytes());
        bytes.extend_from_slice(&DDPF_FOURCC.to_le_bytes());
        bytes.extend_from_slice(b"DXT1");
        bytes.extend_from_slice(&[0u8; 20]); // bit count and the four masks
        bytes.extend_from_slice(&DDSCAPS_TEXTURE.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 4 + 12]); // caps2, caps3, caps4, reserved
        // One block: colour0 is pure red, colour1 pure blue, and every pixel
        // index is 0, which selects colour0.
        bytes.extend_from_slice(&0xF800u16.to_le_bytes());
        bytes.extend_from_slice(&0x001Fu16.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 4]);
        bytes
    }

    /// SVG is rasterised, so the picture has to come out with the colours the
    /// file describes: a blue sky at the top, a dark ground at the bottom.
    #[test]
    fn decodes_an_svg() {
        let decoded =
            decode(Path::new("test-images/landscape.svg"), MAX_TEXTURE_EDGE).expect("svg");
        assert_eq!(decoded.format, "SVG");
        assert_eq!((decoded.source_width, decoded.source_height), (480, 360));
        assert!(
            decoded.width >= 480,
            "the raster should be at least as wide as the drawing, got {}",
            decoded.width
        );
        assert_eq!(
            decoded.pixels.len(),
            (decoded.width * decoded.height * 4) as usize
        );

        let pixel = |x: u32, y: u32| {
            let index = ((y * decoded.width + x) * 4) as usize;
            [
                decoded.pixels[index],
                decoded.pixels[index + 1],
                decoded.pixels[index + 2],
            ]
        };
        let sky = pixel(10, 10);
        // The top gradient stop is #1E3A8A, a deep blue.
        assert!(
            sky[2] > sky[0] + 50 && sky[2] > 100,
            "the sky should be blue, got {sky:?}"
        );
        let ground = pixel(10, decoded.height - 5);
        assert!(
            ground[0] < 40 && ground[1] < 40 && ground[2] < 60,
            "the ground should be dark, got {ground:?}"
        );
        println!("svg: {}x{} rendered from 480x360", decoded.width, decoded.height);
    }

    /// The PSD fixture is written by `tools/make_psd.py`, so this also checks
    /// that the reader agrees with an independent implementation.
    #[test]
    fn decodes_a_psd() {
        let decoded =
            decode(Path::new("test-images/artwork.psd"), MAX_TEXTURE_EDGE).expect("psd");
        assert_eq!(decoded.format, "PSD");
        assert_eq!((decoded.source_width, decoded.source_height), (320, 240));
        assert_eq!((decoded.width, decoded.height), (320, 240));
        assert_eq!(decoded.pixels.len(), 320 * 240 * 4);

        let alpha_at = |x: u32, y: u32| decoded.pixels[((y * 320 + x) * 4 + 3) as usize];
        // The centre disc is opaque, the corners are half transparent.
        assert_eq!(alpha_at(160, 120), 255);
        assert_eq!(alpha_at(2, 2), 128);
        // The gradient runs from a dim red to a bright one left to right.
        let left = decoded.pixels[(120 * 320 * 4) as usize];
        let right = decoded.pixels[(120 * 320 * 4 + 300 * 4) as usize];
        assert!(right > left, "the gradient should brighten to the right");
        println!(
            "psd: {}x{} in {:.1} ms",
            decoded.width, decoded.height, decoded.decode_ms
        );
    }
}

#[cfg(test)]
mod animation_tests {
    use super::*;

    fn colour_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
        let index = ((y * width + x) * 4) as usize;
        [pixels[index], pixels[index + 1], pixels[index + 2]]
    }

    #[test]
    fn decodes_every_frame_of_a_gif() {
        let decoded = decode(Path::new("test-images/animation.gif"), MAX_TEXTURE_EDGE)
            .expect("gif should decode");

        assert_eq!(decoded.format, "GIF");
        assert_eq!((decoded.width, decoded.height), (96, 64));
        assert_eq!(decoded.frames.len(), 3, "all three frames should be kept");
        assert_eq!(decoded.pixels.len(), 96 * 64 * 4, "first frame is a full canvas");

        for frame in &decoded.frames {
            assert!(
                (100..=140).contains(&frame.delay_ms),
                "frame delay {} ms is not the 120 ms that was written",
                frame.delay_ms
            );
        }

        // The coloured square moves 24 px to the right in each frame.
        assert_eq!(colour_at(&decoded.pixels, 96, 12, 28), [220, 40, 40]);
        assert_eq!(colour_at(&decoded.frames[1].pixels, 96, 36, 28), [40, 200, 90]);
        assert_eq!(colour_at(&decoded.frames[2].pixels, 96, 60, 28), [40, 90, 220]);
    }

    #[test]
    fn decodes_every_frame_of_an_apng() {
        // The same animation is checked under both names it is found with: the
        // `.apng` extension some tools write, and `.png` as browsers save it.
        // Which one is used must not matter, because the format is detected
        // from the `acTL` chunk rather than from the name.
        let source = std::fs::read("test-images/animation.apng").expect("fixture");
        let folder = std::env::temp_dir().join("lumen-apng-test");
        std::fs::create_dir_all(&folder).expect("temp folder");
        let as_png = folder.join("animation.png");
        std::fs::write(&as_png, &source).expect("copy");

        for path in [Path::new("test-images/animation.apng"), as_png.as_path()] {
            let decoded = decode(path, MAX_TEXTURE_EDGE).expect("apng should decode");
            assert_eq!(decoded.format, "APNG", "{}", path.display());
            assert_eq!((decoded.width, decoded.height), (96, 64));
            assert_eq!(decoded.frames.len(), 3, "all three frames should be kept");

            for frame in &decoded.frames {
                assert!(
                    (100..=140).contains(&frame.delay_ms),
                    "frame delay {} ms is not the 120 ms that was written",
                    frame.delay_ms
                );
            }

            assert_eq!(colour_at(&decoded.pixels, 96, 12, 28), [220, 40, 40]);
            assert_eq!(colour_at(&decoded.frames[1].pixels, 96, 36, 28), [40, 200, 90]);
            assert_eq!(colour_at(&decoded.frames[2].pixels, 96, 60, 28), [40, 90, 220]);
        }

        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_still_png_is_not_treated_as_an_animation() {
        let decoded =
            decode(Path::new("test-images/alpha-gradient.png"), MAX_TEXTURE_EDGE).expect("png");
        assert!(decoded.frames.is_empty());
        assert_eq!(decoded.format, "PNG");
    }
}
