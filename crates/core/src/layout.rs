//! Layout properties (auto layout, child sizing, constraints, grids).
//! The solver is a separate pure function added in phase 2.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LayoutDirection {
    #[default]
    Horizontal,
    Vertical,
    Wrap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MainAlign {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CrossAlign {
    #[default]
    Start,
    Center,
    End,
    Baseline,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ContainerSizing {
    #[default]
    Fixed,
    Hug,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct AutoLayout {
    pub direction: LayoutDirection,
    pub gap: f32,
    pub cross_gap: f32,
    /// top, right, bottom, left
    pub padding: [f32; 4],
    pub main_align: MainAlign,
    pub cross_align: CrossAlign,
    pub primary_sizing: ContainerSizing,
    pub counter_sizing: ContainerSizing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ChildSizing {
    #[default]
    Fixed,
    Hug,
    Fill,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct LayoutChild {
    pub h_sizing: ChildSizing,
    pub v_sizing: ChildSizing,
    /// Ignores the parent's auto layout.
    pub absolute: bool,
    /// Min/max sizes; rare, so boxed.
    pub limits: Option<Box<SizeLimits>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SizeLimits {
    /// width, height
    pub min: [Option<f32>; 2],
    pub max: [Option<f32>; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Constraint {
    #[default]
    Min,
    Max,
    Center,
    Stretch,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Constraints {
    pub horizontal: Constraint,
    pub vertical: Constraint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridKind {
    Columns,
    Rows,
    Grid,
}

/// What a layout grid draws inside a `w`×`h` frame: filled bands (columns/rows) as
/// (x, y, w, h), and lines (square grids) as (x0, y0, x1, y1).
pub fn grid_geometry(g: &LayoutGrid, w: f32, h: f32) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
    let (mut bands, mut lines) = (Vec::new(), Vec::new());
    let count = f32::from(g.count.max(1));
    match g.kind {
        GridKind::Columns | GridKind::Rows => {
            let along = if g.kind == GridKind::Columns { w } else { h };
            let size = g.size.unwrap_or_else(|| {
                ((along - 2.0 * g.margin - (count - 1.0) * g.gutter) / count).max(0.0)
            });
            // Fixed tracks are centred; stretched ones start at the margin.
            let total = size * count + g.gutter * (count - 1.0);
            let start = if g.size.is_some() {
                (along - total) / 2.0
            } else {
                g.margin
            };
            for i in 0..g.count.max(1) {
                let a = start + f32::from(i) * (size + g.gutter);
                bands.push(if g.kind == GridKind::Columns {
                    [a, 0.0, size, h]
                } else {
                    [0.0, a, w, size]
                });
            }
        }
        GridKind::Grid => {
            let step = g.size.unwrap_or(10.0).max(1.0);
            let mut x = step;
            while x < w {
                lines.push([x, 0.0, x, h]);
                x += step;
            }
            let mut y = step;
            while y < h {
                lines.push([0.0, y, w, y]);
                y += step;
            }
        }
    }
    (bands, lines)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutGrid {
    pub kind: GridKind,
    pub count: u16,
    pub gutter: f32,
    pub margin: f32,
    /// Fixed cell size for `GridKind::Grid` or fixed tracks; `None` = stretch.
    pub size: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretched_columns_fill_between_margins() {
        let g = LayoutGrid {
            kind: GridKind::Columns,
            count: 4,
            gutter: 20.0,
            margin: 40.0,
            size: None,
        };
        let (bands, _) = grid_geometry(&g, 1000.0, 500.0);
        assert_eq!(bands.len(), 4);
        assert_eq!(bands[0], [40.0, 0.0, 215.0, 500.0]);
        assert_eq!(bands[3][0] + bands[3][2], 960.0);
    }
}
