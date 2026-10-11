//! Pure, bounded binary split layout. Zero-sized leaves are hidden, not removed.
use crate::session::PaneId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Columns,
    Rows,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub cols: u16,
    pub rows: u16,
}

impl Rect {
    pub fn visible(self) -> bool {
        self.cols > 0 && self.rows > 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Layout {
    Leaf(PaneId),
    Split(Axis, Box<Layout>, Box<Layout>),
}

impl Layout {
    pub fn split(&mut self, target: PaneId, new: PaneId, axis: Axis) -> bool {
        match self {
            Self::Leaf(id) if *id == target => {
                *self = Self::Split(
                    axis,
                    Box::new(Self::Leaf(target)),
                    Box::new(Self::Leaf(new)),
                );
                true
            }
            Self::Leaf(_) => false,
            Self::Split(_, a, b) => a.split(target, new, axis) || b.split(target, new, axis),
        }
    }

    pub fn remove(self, target: PaneId) -> Option<Self> {
        match self {
            Self::Leaf(id) => (id != target).then_some(Self::Leaf(id)),
            Self::Split(axis, a, b) => match (a.remove(target), b.remove(target)) {
                (Some(a), Some(b)) => Some(Self::Split(axis, Box::new(a), Box::new(b))),
                (a, b) => a.or(b),
            },
        }
    }

    pub fn rectangles(&self, rect: Rect, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            Self::Leaf(id) => out.push((*id, rect)),
            Self::Split(axis, a, b) => {
                let length = match axis {
                    Axis::Columns => rect.cols,
                    Axis::Rows => rect.rows,
                };
                // Reserve one separator only when both children have room.
                let gap = u16::from(length >= 3);
                let first = (length - gap).div_ceil(2);
                let second = length - gap - first;
                let (mut ra, mut rb) = (rect, rect);
                match axis {
                    Axis::Columns => {
                        ra.cols = first;
                        rb.x += first + gap;
                        rb.cols = second;
                    }
                    Axis::Rows => {
                        ra.rows = first;
                        rb.y += first + gap;
                        rb.rows = second;
                    }
                }
                a.rectangles(ra, out);
                b.rectangles(rb, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_and_nested_layouts_stay_inside_bounds() {
        let mut layout = Layout::Leaf(PaneId(0));
        for id in 1..16 {
            assert!(layout.split(PaneId(id - 1), PaneId(id), Axis::Columns));
        }
        for cols in 0..40 {
            for rows in 0..4 {
                let mut rects = Vec::new();
                layout.rectangles(
                    Rect {
                        cols,
                        rows,
                        ..Rect::default()
                    },
                    &mut rects,
                );
                assert_eq!(rects.len(), 16);
                for (_, r) in &rects {
                    assert!(r.x + r.cols <= cols && r.y + r.rows <= rows);
                }
                for (i, (_, a)) in rects.iter().enumerate() {
                    for (_, b) in &rects[i + 1..] {
                        assert!(
                            !a.visible()
                                || !b.visible()
                                || a.x + a.cols <= b.x
                                || b.x + b.cols <= a.x
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn removal_collapses_parent() {
        let mut layout = Layout::Leaf(PaneId(0));
        layout.split(PaneId(0), PaneId(1), Axis::Rows);
        assert_eq!(layout.remove(PaneId(0)), Some(Layout::Leaf(PaneId(1))));
    }
}
