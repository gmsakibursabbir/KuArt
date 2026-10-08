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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutGrid {
    pub kind: GridKind,
    pub count: u16,
    pub gutter: f32,
    pub margin: f32,
    /// Fixed cell size for `GridKind::Grid` or fixed tracks; `None` = stretch.
    pub size: Option<f32>,
}
