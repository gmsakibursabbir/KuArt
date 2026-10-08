//! Reproducible synthetic documents for benchmarks and tests (docs/benchmark-plan.md §2).
//!
//! Same `(nodes, seed)` → same document → byte-identical `.design` output.

use std::sync::Arc;

use ude_core::assets::{AssetEntry, AssetKind, AssetSource, Codec, ContentHash};
use ude_core::geometry::{Affine, FillRule, PathBuilder};
use ude_core::scene::{VectorData, VectorPath};
use ude_core::styles::{Color, Effect, Paint, Shadow};
use ude_core::typography::{
    LetterSpacing, LineHeight, TextCase, TextData, TextDecoration, TextStyle,
};
use ude_core::{AssetId, Document, Node, NodeId, NodeKind, Paints, Point};

/// SplitMix64: tiny, fast, good enough for test data, and stable across platforms.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (n > 0).
    pub fn below(&mut self, n: u32) -> u32 {
        (((self.next_u64() >> 32) * u64::from(n)) >> 32) as u32
    }

    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        let unit = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
        lo + unit * (hi - lo)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SynthSpec {
    /// Nodes under the page (root and page are extra).
    pub nodes: u32,
    pub seed: u64,
}

const PER_FRAME: u32 = 100;
const GROUP_SIZE: u32 = 5;
const WORDS: &[&str] = &[
    "Build", "better", "websites", "Get", "started", "Pricing", "Features", "About", "Contact",
    "Sign", "up", "Learn", "more", "Design", "faster", "Ship", "today",
];

struct Palette {
    fills: Vec<Paints>,
    text_styles: Vec<Arc<[TextStyle]>>,
}

fn palette(doc: &mut Document) -> Palette {
    // A real UI file reuses a handful of colours thousands of times; share them like an importer would.
    let colors = [
        0x2563EBu32,
        0x111827,
        0xF9FAFB,
        0x10B981,
        0xF59E0B,
        0xEF4444,
        0x8B5CF6,
        0xE5E7EB,
        0x1F2937,
        0xFFFFFF,
        0x6B7280,
        0xDBEAFE,
    ];
    let fills: Vec<Paints> = colors
        .iter()
        .map(|&c| {
            Paints::new(vec![Paint::solid(Color::from_rgba8(
                (c >> 16) as u8,
                (c >> 8) as u8,
                c as u8,
                255,
            ))])
        })
        .collect();
    let families = [doc.strings.intern("Inter"), doc.strings.intern("Roboto")];
    let text_styles = [
        (32.0, 700),
        (24.0, 600),
        (16.0, 400),
        (14.0, 500),
        (12.0, 400),
    ]
    .iter()
    .enumerate()
    .map(|(i, &(size, weight))| {
        Arc::from([TextStyle {
            family: families[i % 2],
            weight,
            italic: false,
            size,
            line_height: LineHeight::Px(size * 1.5),
            letter_spacing: LetterSpacing::Normal,
            decoration: TextDecoration::None,
            case: TextCase::Original,
            fills: fills[1].clone(),
        }])
    })
    .collect();
    Palette { fills, text_styles }
}

/// Distribution: 35% rect, 10% ellipse, 20% text, 20% vector, 10% group/frame, 5% raster.
pub fn generate(spec: SynthSpec) -> Document {
    let mut rng = Rng::new(spec.seed);
    let mut doc = Document::new();
    doc.meta.title = format!("synthetic-{}-{}", spec.nodes, spec.seed);
    let pal = palette(&mut doc);

    let image_count = (spec.nodes / 200).max(1);
    let images: Vec<AssetId> = (0..image_count)
        .map(|i| fake_image(&mut doc, &mut rng, i))
        .collect();

    let page_name = doc.strings.intern("Page 1");
    let root = doc.root();
    let page = add(&mut doc, root, Node::new(NodeKind::Page, page_name));

    let frame_name = doc.strings.intern("Frame");
    let group_name = doc.strings.intern("Group");
    let names = ["Rectangle", "Ellipse", "Text", "Vector", "Image"].map(|n| doc.strings.intern(n));

    let mut frame = page;
    let mut group: Option<(NodeId, u32)> = None;
    for i in 0..spec.nodes {
        if i % PER_FRAME == 0 {
            let col = (i / PER_FRAME) % 10;
            let row = (i / PER_FRAME) / 10;
            let f = Node::new(NodeKind::Frame, frame_name)
                .with_transform(Affine::translate(col as f32 * 1600.0, row as f32 * 1200.0))
                .with_size(1440.0, 1024.0)
                .with_fills(pal.fills[9].clone());
            frame = add(&mut doc, page, f);
            group = None;
            continue;
        }
        let parent = match group {
            Some((g, left)) if left > 0 => {
                group = Some((g, left - 1));
                g
            }
            _ => frame,
        };
        let pos = Affine::translate(rng.range(0.0, 1300.0), rng.range(0.0, 900.0));
        let fill = pal.fills[rng.below(pal.fills.len() as u32) as usize].clone();
        let roll = rng.below(100);
        let mut node = match roll {
            0..35 => {
                let mut n = Node::new(NodeKind::Rectangle, names[0])
                    .with_size(rng.range(20.0, 300.0), rng.range(20.0, 200.0));
                n.corner_radius = rng.range(0.0, 16.0);
                n.with_fills(fill)
            }
            35..45 => Node::new(NodeKind::Ellipse(None), names[1])
                .with_size(rng.range(10.0, 120.0), rng.range(10.0, 120.0))
                .with_fills(fill),
            45..65 => {
                let words = 1 + rng.below(8);
                let text: Vec<&str> = (0..words)
                    .map(|_| WORDS[rng.below(WORDS.len() as u32) as usize])
                    .collect();
                let style =
                    pal.text_styles[rng.below(pal.text_styles.len() as u32) as usize].clone();
                let content = text.join(" ");
                let mut data = TextData::plain(&content, style[0].clone());
                data.styles = style;
                Node::new(NodeKind::Text(Box::new(data)), names[2])
                    .with_size(rng.range(60.0, 400.0), 24.0)
            }
            65..85 => Node::new(NodeKind::Vector(random_path(&mut rng)), names[3])
                .with_size(100.0, 100.0)
                .with_fills(fill),
            85..95 => {
                group = Some((
                    add(
                        &mut doc,
                        parent,
                        Node::new(NodeKind::Group, group_name).with_transform(pos),
                    ),
                    GROUP_SIZE,
                ));
                continue;
            }
            _ => {
                let asset = images[rng.below(image_count) as usize];
                Node::new(NodeKind::Raster { asset }, names[4])
                    .with_size(rng.range(80.0, 600.0), rng.range(60.0, 400.0))
            }
        };
        node.transform = pos;
        if rng.below(20) == 0 {
            node.ext_mut().effects = Box::new([Effect::DropShadow(Shadow {
                color: Color::from_rgba8(0, 0, 0, 40),
                offset: Point::new(0.0, 4.0),
                blur: 12.0,
                spread: 0.0,
                blend: Default::default(),
                visible: true,
            })]);
        }
        add(&mut doc, parent, node);
    }
    let _ = doc.take_changes();
    doc
}

fn add(doc: &mut Document, parent: NodeId, node: Node) -> NodeId {
    doc.build_child(parent, node)
        .expect("generator builds a valid tree")
}

fn random_path(rng: &mut Rng) -> Arc<VectorData> {
    let mut b = PathBuilder::default();
    b.move_to(rng.range(0.0, 100.0), rng.range(0.0, 100.0));
    for _ in 0..4 + rng.below(5) {
        b.cubic_to(
            rng.range(0.0, 100.0),
            rng.range(0.0, 100.0),
            rng.range(0.0, 100.0),
            rng.range(0.0, 100.0),
            rng.range(0.0, 100.0),
            rng.range(0.0, 100.0),
        );
    }
    b.close();
    Arc::new(VectorData {
        paths: Box::new([VectorPath {
            path: b.build(),
            fill_rule: FillRule::NonZero,
        }]),
    })
}

/// Placeholder pixel payload (not a real PNG): sized like a small thumbnail,
/// so file-size numbers stay meaningful without bundling image fixtures.
fn fake_image(doc: &mut Document, rng: &mut Rng, i: u32) -> AssetId {
    let bytes: Vec<u8> = (0..4096).map(|_| (rng.next_u64() & 0x0f) as u8).collect();
    let mut hash = [0u8; 32];
    hash[..4].copy_from_slice(&i.to_le_bytes());
    hash[4..12].copy_from_slice(&rng.next_u64().to_le_bytes());
    doc.assets.insert(AssetEntry {
        hash: ContentHash(hash),
        kind: AssetKind::Image {
            width: 32,
            height: 32,
        },
        source: AssetSource::Memory {
            bytes: bytes.into(),
            codec: Codec::Raw,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_node_count_and_deterministic() {
        let a = generate(SynthSpec {
            nodes: 1000,
            seed: 7,
        });
        // root + page + requested nodes
        assert_eq!(a.descendants(a.root()).count(), 1002);
        let b = generate(SynthSpec {
            nodes: 1000,
            seed: 7,
        });
        let names = |d: &Document| {
            d.descendants(d.root())
                .map(|id| d.node(id).unwrap().transform)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&a), names(&b));
    }
}
