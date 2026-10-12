//! Raster image import (docs/format-strategy.md §7): a page with one frame sized to the
//! image and a raster layer inside it. Only the header is read; pixels stay encoded
//! until the renderer decodes them at the size it needs.

use std::io::Cursor;

use ude_core::assets::{AssetEntry, AssetKind, AssetSource, Codec, ContentHash};
use ude_core::{Document, Node, NodeKind, SourceFormat};

/// Images above this many pixels are refused (untrusted-input guard).
pub const MAX_PIXELS: u64 = 300_000_000;

#[derive(Debug)]
pub enum RasterError {
    Unsupported,
    Decode(String),
    TooLarge(u32, u32),
}

impl std::fmt::Display for RasterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RasterError::Unsupported => write!(f, "not a PNG, JPEG or WebP image"),
            RasterError::Decode(e) => write!(f, "cannot read image: {e}"),
            RasterError::TooLarge(w, h) => write!(f, "image is too large ({w}x{h})"),
        }
    }
}

impl std::error::Error for RasterError {}

pub fn import(bytes: Vec<u8>, name: &str) -> Result<Document, RasterError> {
    let reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| RasterError::Decode(e.to_string()))?;
    let codec = match reader.format() {
        Some(image::ImageFormat::Png) => Codec::Png,
        Some(image::ImageFormat::Jpeg) => Codec::Jpeg,
        Some(image::ImageFormat::WebP) => Codec::WebP,
        _ => return Err(RasterError::Unsupported),
    };
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| RasterError::Decode(e.to_string()))?;
    if u64::from(w) * u64::from(h) > MAX_PIXELS || w == 0 || h == 0 {
        return Err(RasterError::TooLarge(w, h));
    }

    let mut doc = Document::new();
    doc.meta.title = name.to_owned();
    doc.meta.source = Some(SourceFormat::Raster);
    let asset = doc.assets.insert(AssetEntry {
        hash: ContentHash(*blake3::hash(&bytes).as_bytes()),
        kind: AssetKind::Image {
            width: w,
            height: h,
        },
        source: AssetSource::Memory {
            bytes: bytes.into(),
            codec,
        },
    });
    let (page_name, layer_name) = (doc.strings.intern("Page 1"), doc.strings.intern(name));
    let root = doc.root();
    let build = |doc: &mut Document| -> Result<(), ude_core::DocError> {
        let page = doc.build_child(root, Node::new(NodeKind::Page, page_name))?;
        let frame = doc.build_child(
            page,
            Node::new(NodeKind::Frame, layer_name).with_size(w as f32, h as f32),
        )?;
        doc.build_child(
            frame,
            Node::new(NodeKind::Raster { asset }, layer_name).with_size(w as f32, h as f32),
        )?;
        Ok(())
    };
    build(&mut doc).map_err(|e| RasterError::Decode(e.to_string()))?;
    let _ = doc.take_changes();
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_png_as_frame_with_raster_layer() {
        let img = image::RgbaImage::from_pixel(30, 20, image::Rgba([1, 2, 3, 255]));
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let doc = import(png, "photo.png").unwrap();
        let page = doc.children(doc.root())[0];
        let frame = doc.children(page)[0];
        let layer = doc.node(doc.children(frame)[0]).unwrap();
        let NodeKind::Raster { asset } = layer.kind else {
            panic!("expected a raster layer")
        };
        assert_eq!((layer.size.w, layer.size.h), (30.0, 20.0));
        assert!(matches!(
            doc.assets.get(asset).map(|e| e.kind),
            Some(AssetKind::Image {
                width: 30,
                height: 20
            })
        ));
    }

    #[test]
    fn rejects_non_images() {
        assert!(import(b"hello world".to_vec(), "x").is_err());
    }
}
