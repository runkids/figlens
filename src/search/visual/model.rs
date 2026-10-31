//! The one pinned SigLIP vision profile. Loaded only in the optional native embedding child.

use std::error::Error;
use std::io::Read;
use std::path::Path;

use candle_core::{DType, Device, Module, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::siglip;
use resvg::tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};
use sha2::{Digest, Sha256};

use super::QueryImage;

pub const MODEL: &str = "google/siglip-base-patch16-224";
pub const REVISION: &str = "7fd15f0689c79d79e38b1c2e2e2370a7bf2761ed";
pub const SHA256: &str = "2c63cb7d1f2e95ba501893cbb8faeb4ea9a3af295498d35097126228659c2af8";
pub const DIM: usize = 768;
pub const PIXELS: usize = 224;
pub const MAX_BATCH: usize = 4;
/// The parent policy is visual-hash-1; then bilinear 224 RGB, scale to [-1,1], pooled L2.
pub const PREPROCESS: &str = "visual-hash-1-bilinear224-rgb-half-pooled-l2-1";
const WEIGHT_BYTES: u64 = 812_672_320;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub struct Model {
    vision: siglip::VisionModel,
}

impl Model {
    /// Safe buffered loading avoids a memory map of user-owned files that may change later.
    /// Only the vision weights survive construction; the text tower and file buffer are dropped.
    pub fn load(dir: &Path) -> Result<Self> {
        let mut data = Vec::new();
        std::fs::File::open(dir.join("model.safetensors"))?
            .take(WEIGHT_BYTES + 1)
            .read_to_end(&mut data)?;
        if data.len() as u64 != WEIGHT_BYTES || super::super::hex(&Sha256::digest(&data)) != SHA256
        {
            return Err(super::super::invalid(
                "visual model checksum mismatch; use the pinned SigLIP profile",
            ));
        }
        let device = Device::Cpu;
        let vb = VarBuilder::from_buffered_safetensors(data, DType::F32, &device)?;
        let cfg = siglip::Config::base_patch16_224();
        let vision = siglip::VisionModel::new(&cfg.vision_config, true, vb.pp("vision_model"))?;
        Ok(Self { vision })
    }

    pub fn embed(&self, images: &[QueryImage]) -> Result<Vec<Vec<f32>>> {
        let input = pixels(images)?;
        let tensor = Tensor::from_vec(input, (images.len(), 3, PIXELS, PIXELS), &Device::Cpu)?;
        let features = self.vision.forward(&tensor)?;
        let norm = features.sqr()?.sum_keepdim(1)?.sqrt()?;
        let vectors = features.broadcast_div(&norm)?.to_vec2::<f32>()?;
        if vectors
            .iter()
            .any(|v| v.len() != DIM || v.iter().any(|x| !x.is_finite()))
        {
            return Err(super::super::invalid(
                "visual runtime returned invalid vectors",
            ));
        }
        Ok(vectors)
    }
}

/// NCHW floats from the already opaque, metadata-free, normalized query raster.
/// Public for reproducible reference parity; callers still pass through `QueryImage` admission.
pub fn pixels(images: &[QueryImage]) -> Result<Vec<f32>> {
    if images.is_empty() || images.len() > MAX_BATCH {
        return Err(super::super::invalid("a visual batch holds 1 to 4 images"));
    }
    let plane = PIXELS * PIXELS;
    let mut out = vec![0.0; images.len() * 3 * plane];
    for (i, image) in images.iter().enumerate() {
        let mut resized = Pixmap::new(PIXELS as u32, PIXELS as u32).unwrap();
        resized.draw_pixmap(
            0,
            0,
            image.pixmap.as_ref(),
            &PixmapPaint {
                quality: FilterQuality::Bilinear,
                ..PixmapPaint::default()
            },
            Transform::from_scale(
                PIXELS as f32 / super::SIZE as f32,
                PIXELS as f32 / super::SIZE as f32,
            ),
            None,
        );
        for (p, rgba) in resized.data().chunks_exact(4).enumerate() {
            for c in 0..3 {
                out[(i * 3 + c) * plane + p] = f32::from(rgba[c]) / 127.5 - 1.0;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use resvg::tiny_skia::Color;

    #[test]
    fn preprocessing_is_bounded_rgb_nchw_and_half_normalized() {
        let mut source = Pixmap::new(32, 32).unwrap();
        source.fill(Color::from_rgba8(255, 0, 0, 255));
        let image = QueryImage::from_pixmap(&source, None).unwrap();
        let v = pixels(&[image]).unwrap();
        assert_eq!(v.len(), 3 * PIXELS * PIXELS);
        assert!(v[..PIXELS * PIXELS].iter().all(|v| *v == 1.0));
        assert!(v[PIXELS * PIXELS..].iter().all(|v| *v == -1.0));
        assert!(pixels(&[]).is_err());
        let too_many: Vec<_> = (0..5)
            .map(|_| QueryImage::from_pixmap(&source, None).unwrap())
            .collect();
        assert!(pixels(&too_many).is_err());
    }
}
