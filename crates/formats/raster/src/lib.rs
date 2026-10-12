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
    let bytes_for_comments = bytes.clone();
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
    doc.comments = image_comments(&bytes_for_comments);
    let _ = doc.take_changes();
    Ok(doc)
}

/// Comments stored in the image: PNG `tEXt`/`iTXt` (Comment, Description, Title; author from
/// Author) and JPEG `COM` segments.
pub fn image_comments(bytes: &[u8]) -> Vec<ude_core::comments::Comment> {
    let mut found: Vec<(String, &str)> = Vec::new();
    let mut author = String::new();
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let mut p = 8;
        while p + 8 <= bytes.len() {
            let len =
                u32::from_be_bytes([bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3]]) as usize;
            let kind = &bytes[p + 4..p + 8];
            let Some(data) = bytes.get(p + 8..p + 8 + len) else {
                break;
            };
            let text = match kind {
                b"tEXt" => data.iter().position(|&b| b == 0).map(|z| {
                    let kw = String::from_utf8_lossy(&data[..z]).into_owned();
                    (
                        kw,
                        data[z + 1..]
                            .iter()
                            .map(|&c| char::from(c))
                            .collect::<String>(),
                    )
                }),
                // Uncompressed iTXt: keyword, 0, flag, method, language, 0, translated, 0, text.
                b"iTXt" => data.iter().position(|&b| b == 0).and_then(|z| {
                    if data.get(z + 1) != Some(&0) {
                        return None;
                    }
                    let rest = data.get(z + 3..)?;
                    let l = rest.iter().position(|&b| b == 0)?;
                    let rest = &rest[l + 1..];
                    let t = rest.iter().position(|&b| b == 0)?;
                    Some((
                        String::from_utf8_lossy(&data[..z]).into_owned(),
                        String::from_utf8_lossy(&rest[t + 1..]).into_owned(),
                    ))
                }),
                b"IEND" => break,
                _ => None,
            };
            if let Some((kw, text)) = text {
                match kw.as_str() {
                    "Author" => author = text,
                    "Comment" => found.push((text, "PNG comment")),
                    "Description" => found.push((text, "PNG description")),
                    "Title" => found.push((text, "PNG title")),
                    _ => {}
                }
            }
            p += 12 + len;
        }
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut p = 2;
        while p + 4 <= bytes.len() && bytes[p] == 0xFF {
            let marker = bytes[p + 1];
            if marker == 0xDA || marker == 0xD9 {
                break; // image data starts: no more metadata segments
            }
            let len = usize::from(u16::from_be_bytes([bytes[p + 2], bytes[p + 3]]));
            let Some(data) = bytes.get(p + 4..p + 2 + len) else {
                break;
            };
            if marker == 0xFE {
                found.push((String::from_utf8_lossy(data).into_owned(), "JPEG comment"));
            }
            p += 2 + len;
        }
    }
    found
        .into_iter()
        .filter(|(t, _)| !t.trim().is_empty())
        .map(|(text, source)| ude_core::comments::Comment {
            text: text.trim().to_owned(),
            author: author.clone(),
            source: source.into(),
            ..Default::default()
        })
        .collect()
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
    fn reads_png_and_jpeg_comments() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255]));
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        // Insert tEXt chunks right after IHDR (8 + 25 bytes).
        let chunk = |kw: &str, text: &str| {
            let data = [kw.as_bytes(), &[0], text.as_bytes()].concat();
            let mut c = (data.len() as u32).to_be_bytes().to_vec();
            c.extend(b"tEXt");
            c.extend(&data);
            c.extend([0, 0, 0, 0]); // CRC is not checked by the reader under test
            c
        };
        let mut with = png[..33].to_vec();
        with.extend(chunk("Author", "Ana"));
        with.extend(chunk("Comment", "Hero image, crop to 16:9"));
        with.extend(&png[33..]);
        let c = image_comments(&with);
        assert_eq!(c.len(), 1);
        assert_eq!(
            (c[0].author.as_str(), c[0].text.as_str()),
            ("Ana", "Hero image, crop to 16:9")
        );

        let jpeg = [
            &[0xFF, 0xD8, 0xFF, 0xFE, 0x00, 0x07][..],
            b"hello",
            &[0xFF, 0xD9],
        ]
        .concat();
        assert_eq!(image_comments(&jpeg)[0].text, "hello");
    }

    #[test]
    fn rejects_non_images() {
        assert!(import(b"hello world".to_vec(), "x").is_err());
    }
}
