use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{App, RenderImage};
use image::{Frame, imageops::FilterType};
use rustc_hash::FxHashMap;
use schema::unique_bytes::UniqueBytes;

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub enum ImageTransformation {
    None,
    Resize {
        width: u32,
        height: u32,
    },
    ResizeToWidth {
        width: u32,
    },
}

struct CacheEntry {
    expiring: Instant,
    value: Option<Arc<RenderImage>>,
}

#[derive(Default)]
struct PngRenderCache {
    map: FxHashMap<(UniqueBytes, ImageTransformation), CacheEntry>,
    submitted_cleanup: bool,
}

impl gpui::Global for PngRenderCache {}

const EXPIRY_SECONDS: u64 = 1;

pub fn render(image: UniqueBytes, cx: &mut App) -> gpui::Img {
    render_with_transform(image, ImageTransformation::None, cx)
}

pub fn render_with_transform(image: UniqueBytes, transform: ImageTransformation, cx: &mut App) -> gpui::Img {
    let cache = cx.default_global::<PngRenderCache>();

    let result = if let Some(result) = cache.get_or_create(image, transform) {
        gpui::img(result)
    } else {
        gpui::img(gpui::ImageSource::Resource(gpui::Resource::Embedded("images/missing.png".into())))
    };

    if !cache.submitted_cleanup {
        cache.submitted_cleanup = true;
        cx.spawn(async |cx| {
            let _ = cx.update_global(|cache: &mut PngRenderCache, cx| {
                let now = Instant::now();
                cache.map.retain(|_, entry| {
                    if now > entry.expiring {
                        if let Some(image) = &entry.value {
                            cx.drop_image(image.clone(), None);
                        }
                        false
                    } else {
                        true
                    }
                });
                cache.submitted_cleanup = false;
            });
        })
        .detach();
    }

    result
}

impl PngRenderCache {
    fn get_or_create(&mut self, image: UniqueBytes, transform: ImageTransformation) -> Option<Arc<RenderImage>> {
        let key = (image, transform);

        if let Some(entry) = self.map.get_mut(&key) {
            entry.expiring = Instant::now() + Duration::from_secs(EXPIRY_SECONDS);
            return entry.value.clone();
        }

        let result = image::load_from_memory_with_format(&key.0, image::ImageFormat::Png).map(|mut image| {
            match transform {
                ImageTransformation::None => {},
                ImageTransformation::Resize { width, height } => {
                    let old_width = image.width();
                    let old_height = image.height();
                    if old_width != width || old_height != height {
                        // ponytail: Triangle is ~3x faster than Lanczos3, visually close for icons
                        let filter = if old_width > width || old_height > height {
                            FilterType::Triangle
                        } else {
                            FilterType::Nearest
                        };
                        image = image.resize(width, height, filter);
                    }
                },
                ImageTransformation::ResizeToWidth { width } => {
                    let old_width = image.width();
                    let old_height = image.height();
                    let ratio = width as f64 / old_width as f64;
                    let height = ((old_height as f64 * ratio).round() as u32).max(1);
                    if old_width != width || old_height != height {
                        // ponytail: Triangle is ~3x faster than Lanczos3, visually close for icons
                        let filter = if old_width > width || old_height > height {
                            FilterType::Triangle
                        } else {
                            FilterType::Nearest
                        };
                        image = image.resize_exact(width, height, filter);
                    }
                },
            }

            let mut data = image.into_rgba8();

            // Convert from RGBA to BGRA.
            for pixel in data.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }

            RenderImage::new([Frame::new(data)])
        });

        let render_image = match result {
            Ok(render_image) => Some(Arc::new(render_image)),
            Err(error) => {
                log::warn!("Error loading png: {error:?}");
                None
            },
        };

        let entry = CacheEntry {
            expiring: Instant::now() + Duration::from_secs(EXPIRY_SECONDS),
            value: render_image.clone(),
        };
        self.map.insert(key, entry);

        render_image
    }
}
