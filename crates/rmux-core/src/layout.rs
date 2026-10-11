pub type PaneId = u64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// Display geometry shared by PTY sizing and rendering.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaneLayout {
    pub headers: bool,
    pub padding_x: u16,
    pub padding_y: u16,
}
impl PaneLayout {
    pub fn new(headers: bool, padding: u16) -> Result<Self, &'static str> {
        if padding > 16 {
            return Err("pane padding exceeds 16 cells");
        }
        Ok(Self {
            headers,
            padding_x: padding,
            padding_y: padding,
        })
    }
    pub fn content(self, rect: Rect) -> Rect {
        let top = u16::from(self.headers).min(rect.height);
        let inset_x = self.padding_x.min(rect.width / 2);
        let inset_y = self.padding_y.min(rect.height.saturating_sub(top) / 2);
        Rect {
            x: rect.x.saturating_add(inset_x),
            y: rect.y.saturating_add(top).saturating_add(inset_y),
            width: rect.width.saturating_sub(self.padding_x.saturating_mul(2)),
            height: rect
                .height
                .saturating_sub(top)
                .saturating_sub(self.padding_y.saturating_mul(2)),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Down,
    Up,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Columns,
    Rows,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Layout {
    Pane(PaneId),
    Split {
        axis: Axis,
        first: Box<Self>,
        second: Box<Self>,
    },
}

impl Layout {
    pub fn split(&mut self, target: PaneId, new: PaneId, axis: Axis) -> bool {
        match self {
            Self::Pane(id) if *id == target => {
                *self = Self::Split {
                    axis,
                    first: Box::new(Self::Pane(target)),
                    second: Box::new(Self::Pane(new)),
                };
                true
            }
            Self::Pane(_) => false,
            Self::Split { first, second, .. } => {
                first.split(target, new, axis) || second.split(target, new, axis)
            }
        }
    }

    /// Consuming the tree lets removal collapse branches without placeholder pane IDs.
    pub fn remove(self, target: PaneId) -> Option<Self> {
        match self {
            Self::Pane(id) => (id != target).then_some(Self::Pane(id)),
            Self::Split {
                axis,
                first,
                second,
            } => match (first.remove(target), second.remove(target)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    axis,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (remaining, None) | (None, remaining) => remaining,
            },
        }
    }

    pub fn rectangles(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        let mut result = Vec::new();
        self.place(area, &mut result);
        result
    }

    fn place(&self, area: Rect, result: &mut Vec<(PaneId, Rect)>) {
        match self {
            Self::Pane(id) => result.push((*id, area)),
            Self::Split {
                axis,
                first,
                second,
            } => {
                let (a, b) = divide(area, *axis);
                first.place(a, result);
                second.place(b, result);
            }
        }
    }
}

pub fn divide(area: Rect, axis: Axis) -> (Rect, Rect) {
    match axis {
        Axis::Columns => {
            let available = area.width.saturating_sub(1);
            let left = available / 2;
            (
                Rect {
                    width: left,
                    ..area
                },
                Rect {
                    x: area.x.saturating_add(left).saturating_add(1),
                    width: available - left,
                    ..area
                },
            )
        }
        Axis::Rows => {
            let available = area.height.saturating_sub(1);
            let top = available / 2;
            (
                Rect {
                    height: top,
                    ..area
                },
                Rect {
                    y: area.y.saturating_add(top).saturating_add(1),
                    height: available - top,
                    ..area
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_splits_and_removal_preserve_other_panes() {
        let mut tree = Layout::Pane(1);
        assert!(tree.split(1, 2, Axis::Columns));
        assert!(tree.split(2, 3, Axis::Rows));
        let area = Rect {
            x: 0,
            y: 0,
            width: 81,
            height: 24,
        };
        let rects = tree.rectangles(area);
        assert_eq!(rects[0].1.width, 40);
        assert_eq!(
            rects[1].1,
            Rect {
                x: 41,
                y: 0,
                width: 40,
                height: 11
            }
        );
        assert_eq!(rects[2].1.y, 12);
        let tree = tree.remove(2).unwrap();
        assert_eq!(
            tree.rectangles(area)[1],
            (
                3,
                Rect {
                    x: 41,
                    y: 0,
                    width: 40,
                    height: 24
                }
            )
        );
        assert_eq!(tree.remove(1), Some(Layout::Pane(3)));
        assert_eq!(Layout::Pane(3).remove(3), None);
    }

    #[test]
    fn tiny_terminals_never_underflow() {
        let mut tree = Layout::Pane(1);
        tree.split(1, 2, Axis::Columns);
        tree.split(2, 3, Axis::Rows);
        for width in 0..4 {
            for height in 0..4 {
                for (_, rect) in tree.rectangles(Rect {
                    width,
                    height,
                    ..Rect::default()
                }) {
                    let _ = PaneLayout::default().content(rect);
                    assert!(rect.width <= width);
                    assert!(rect.height <= height);
                }
            }
        }
    }
}
