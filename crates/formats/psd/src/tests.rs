//! Tests build small PSD files with a minimal writer (raw and RLE channels, groups, masks,
//! clipping), then check the imported tree and decoded pixels.

use super::*;

struct TestLayer {
    name: &'static str,
    /// top, left, bottom, right
    rect: [i32; 4],
    rgba: [u8; 4],
    rle: bool,
    section: Option<u32>,
    hidden: bool,
    opacity: u8,
    blend: &'static [u8; 4],
    clipping: bool,
    /// Mask covering the layer rect: left half black (hidden), right half white.
    half_mask: bool,
}

impl TestLayer {
    fn pixel(name: &'static str, rect: [i32; 4], rgba: [u8; 4]) -> Self {
        Self {
            name,
            rect,
            rgba,
            rle: false,
            section: None,
            hidden: false,
            opacity: 255,
            blend: b"norm",
            clipping: false,
            half_mask: false,
        }
    }
    fn section(name: &'static str, kind: u32) -> Self {
        Self {
            section: Some(kind),
            blend: b"pass",
            ..Self::pixel(name, [0, 0, 0, 0], [0; 4])
        }
    }
}

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}
fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

/// PackBits with literal runs only (valid, not optimal).
fn pack_row(row: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in row.chunks(128) {
        out.push((chunk.len() - 1) as u8);
        out.extend_from_slice(chunk);
    }
    out
}

fn channel(plane: &[u8], w: usize, h: usize, rle: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if rle {
        out.extend(be16(1));
        let rows: Vec<Vec<u8>> = plane.chunks(w.max(1)).take(h).map(pack_row).collect();
        for r in &rows {
            out.extend(be16(r.len() as u16));
        }
        for r in rows {
            out.extend(r);
        }
    } else {
        out.extend(be16(0));
        out.extend_from_slice(plane);
    }
    out
}

fn psd(width: u32, height: u32, layers: &[TestLayer]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend(b"8BPS");
    f.extend(be16(1));
    f.extend([0; 6]);
    f.extend(be16(3));
    f.extend(be32(height));
    f.extend(be32(width));
    f.extend(be16(8));
    f.extend(be16(3));
    f.extend(be32(0)); // colour mode data
    f.extend(be32(0)); // image resources

    let mut li = Vec::new();
    let mut data = Vec::new();
    if !layers.is_empty() {
        li.extend(be16(layers.len() as u16));
        for l in layers {
            let [t, le, b, r] = l.rect;
            let (w, h) = ((r - le) as usize, (b - t) as usize);
            for v in l.rect {
                li.extend(be32(v as u32));
            }
            let mut chans: Vec<(i16, Vec<u8>)> = [-1i16, 0, 1, 2]
                .iter()
                .map(|&id| {
                    let v = l.rgba[if id < 0 { 3 } else { id as usize }];
                    (id, channel(&vec![v; w * h], w, h, l.rle))
                })
                .collect();
            if l.half_mask {
                let plane: Vec<u8> = (0..w * h)
                    .map(|i| if i % w < w / 2 { 0 } else { 255 })
                    .collect();
                chans.push((-2, channel(&plane, w, h, false)));
            }
            li.extend(be16(chans.len() as u16));
            for (id, c) in &chans {
                li.extend(be16(*id as u16));
                li.extend(be32(c.len() as u32));
            }
            li.extend(b"8BIM");
            li.extend(l.blend);
            li.extend([
                l.opacity,
                u8::from(l.clipping),
                if l.hidden { 2 } else { 0 },
                0,
            ]);
            let mut extra = Vec::new();
            if l.half_mask {
                extra.extend(be32(20));
                for v in l.rect {
                    extra.extend(be32(v as u32));
                }
                extra.extend([0, 0, 0, 0]); // default colour, flags, padding
            } else {
                extra.extend(be32(0));
            }
            extra.extend(be32(0)); // blending ranges
            let name = l.name.as_bytes();
            extra.push(name.len() as u8);
            extra.extend(name);
            let mut padded = name.len() + 1;
            while padded % 4 != 0 {
                extra.push(0);
                padded += 1;
            }
            if let Some(kind) = l.section {
                extra.extend(b"8BIM");
                extra.extend(b"lsct");
                extra.extend(be32(4));
                extra.extend(be32(kind));
            }
            li.extend(be32(extra.len() as u32));
            li.extend(extra);
            for (_, c) in chans {
                data.extend(c);
            }
        }
        li.extend(data);
        if li.len() % 2 == 1 {
            li.push(0);
        }
    }
    if layers.is_empty() {
        f.extend(be32(0));
    } else {
        f.extend(be32((4 + li.len() + 4) as u32));
        f.extend(be32(li.len() as u32));
        f.extend(&li);
        f.extend(be32(0)); // global mask info
    }
    // Merged image: raw, mid grey.
    f.extend(be16(0));
    f.extend(vec![128u8; (width * height * 3) as usize]);
    f
}

fn rgba_of(doc: &Document, id: ude_core::NodeId) -> (u32, u32, Vec<u8>) {
    let NodeKind::Raster { asset } = doc.node(id).unwrap().kind else {
        panic!("not a raster")
    };
    let AssetSource::Memory { bytes, codec } = &doc.assets.get(asset).unwrap().source else {
        panic!("not in memory")
    };
    match codec {
        Codec::PsdChannels => decode_rgba(bytes).unwrap(),
        Codec::Png => {
            let img = image::load_from_memory(bytes).unwrap().into_rgba8();
            (img.width(), img.height(), img.into_raw())
        }
        c => panic!("unexpected codec {c:?}"),
    }
}

fn canvas(doc: &Document) -> ude_core::NodeId {
    doc.children(doc.children(doc.root())[0])[0]
}

#[test]
fn flat_file_uses_merged_image() {
    let im = import(&psd(4, 3, &[]), "flat.psd").unwrap();
    let c = canvas(&im.doc);
    let layer = im.doc.children(c)[0];
    let (w, h, px) = rgba_of(&im.doc, layer);
    assert_eq!((w, h), (4, 3));
    assert_eq!(&px[..4], &[128, 128, 128, 255]);
}

#[test]
fn layers_groups_masks_and_clipping() {
    let mut blue = TestLayer::pixel("Blue", [2, 2, 6, 10], [0, 0, 255, 255]);
    blue.rle = true;
    blue.half_mask = true;
    blue.blend = b"mul ";
    blue.opacity = 128;
    let mut clipped = TestLayer::pixel("Clipped", [0, 0, 10, 20], [0, 255, 0, 255]);
    clipped.clipping = true;
    let mut hidden = TestLayer::pixel("Hidden", [0, 0, 1, 1], [9, 9, 9, 255]);
    hidden.hidden = true;
    let layers = [
        TestLayer::pixel("Background", [0, 0, 10, 10], [255, 0, 0, 255]),
        clipped,
        TestLayer::section("</Layer group>", 3),
        blue,
        hidden,
        TestLayer::section("Icons", 1),
    ];
    let im = import(&psd(16, 10, &layers), "layers.psd").unwrap();
    let doc = &im.doc;
    let c = canvas(doc);
    assert_eq!(doc.node(c).unwrap().size, ude_core::Size::new(16.0, 10.0));
    let kids = doc.children(c);
    assert_eq!(kids.len(), 3, "background, clipped layer, group");
    let name = |id| doc.strings.resolve(doc.node(id).unwrap().name).to_owned();
    assert_eq!(name(kids[0]), "Background");
    assert_eq!(name(kids[2]), "Icons");

    // Clipped layer (20 wide) keeps only the pixels over the 10-wide background.
    let (w, _, px) = rgba_of(doc, kids[1]);
    assert_eq!(w, 20);
    assert_eq!(px[3], 255, "inside the base");
    assert_eq!(px[15 * 4 + 3], 0, "outside the base");

    // Group: pass-through, bounds of its layers, children relative to it.
    let group = doc.node(kids[2]).unwrap();
    assert_eq!(group.blend, BlendMode::PassThrough);
    assert_eq!(
        group.transform.translation(),
        ude_core::Point::new(0.0, 0.0)
    );
    let gk = doc.children(kids[2]);
    assert_eq!(gk.len(), 2);
    let b = doc.node(gk[0]).unwrap();
    assert_eq!(b.blend, BlendMode::Multiply);
    assert!((b.opacity - 128.0 / 255.0).abs() < 1e-3);
    assert_eq!(b.transform.translation(), ude_core::Point::new(2.0, 2.0));
    let (w, h, px) = rgba_of(doc, gk[0]);
    assert_eq!((w, h), (8, 4));
    assert_eq!(&px[..4], &[0, 0, 255, 0], "left half masked out");
    assert_eq!(&px[7 * 4..8 * 4], &[0, 0, 255, 255], "right half visible");
    assert!(!doc.node(gk[1]).unwrap().flags.contains(NodeFlags::VISIBLE));
    assert!(im.warnings.is_empty(), "{:?}", im.warnings);
}

#[test]
fn rejects_bad_input() {
    assert!(import(b"nope", "x").is_err());
    let mut f = psd(2, 2, &[]);
    f.truncate(30);
    assert!(import(&f, "x").is_err());
    // Huge dimensions are refused before allocating.
    let mut big = psd(2, 2, &[]);
    big[14..18].copy_from_slice(&be32(300_000));
    big[18..22].copy_from_slice(&be32(300_000));
    assert!(import(&big, "x").is_err());
}

#[test]
fn packbits_runs() {
    let mut out = Vec::new();
    // Repeat 0xAA 3 times, then literal [1, 2].
    channels_unpack(&[0xFE, 0xAA, 0x01, 1, 2], 5, &mut out);
    assert_eq!(out, [0xAA, 0xAA, 0xAA, 1, 2]);
}

fn channels_unpack(src: &[u8], len: usize, out: &mut Vec<u8>) {
    let plane = channels::decode_plane(
        1,
        &[&be16(src.len() as u16)[..], src].concat(),
        len as u32,
        1,
        8,
        false,
    )
    .unwrap();
    out.extend(plane);
}

#[test]
fn photoshop_notes_become_comments() {
    // A flat file whose layer section holds only a global `Anno` block.
    let mut note = Vec::new();
    note.extend(b"txtA");
    note.extend([1, 0, 0, 0]); // open, flags, optional blocks
    for v in [40i32, 30, 60, 50] {
        note.extend(v.to_be_bytes()); // icon rect: top, left, bottom, right
    }
    note.extend([0u8; 16 + 10]); // popup rect, colour
    for s in ["Ana", "Note", "D:2026"] {
        note.push(s.len() as u8);
        note.extend(s.as_bytes());
        if s.len() % 2 == 0 {
            note.push(0);
        }
    }
    let text: Vec<u8> = [0xFE, 0xFF]
        .into_iter()
        .chain(
            "Tighten the kerning"
                .encode_utf16()
                .flat_map(u16::to_be_bytes),
        )
        .collect();
    note.extend(((text.len() + 4) as u32).to_be_bytes());
    note.extend(b"txtC");
    note.extend(&text);
    let mut anno = vec![0, 2, 0, 1];
    anno.extend(1u32.to_be_bytes());
    anno.extend(((note.len() + 4) as u32).to_be_bytes());
    anno.extend(&note);

    let mut block = Vec::new();
    block.extend(b"8BIM");
    block.extend(b"Anno");
    block.extend((anno.len() as u32).to_be_bytes());
    block.extend(&anno);
    while block.len() % 4 != 0 {
        block.push(0);
    }

    let mut f = psd(4, 3, &[]);
    // Replace the empty layer section (u32 0 at offset 34) with: info len 0, mask len 0, blocks.
    let mut section = Vec::new();
    section.extend(0u32.to_be_bytes());
    section.extend(0u32.to_be_bytes());
    section.extend(&block);
    let mut out = f[..34].to_vec();
    out.extend((section.len() as u32).to_be_bytes());
    out.extend(&section);
    out.extend(f.split_off(38));
    let im = import(&out, "notes.psd").unwrap();
    assert_eq!(im.doc.comments.len(), 1);
    let c = &im.doc.comments[0];
    assert_eq!(
        (c.author.as_str(), c.text.as_str()),
        ("Ana", "Tighten the kerning")
    );
    assert_eq!((c.x, c.y), (30.0, 40.0));
}
