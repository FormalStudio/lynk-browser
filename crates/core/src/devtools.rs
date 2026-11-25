//! Where a window shows DevTools: docked below or beside the page, or in a window of their own,
//! and how much of the page area a docked DevTools leaves the page on each side. One per window,
//! so DevTools come back where they were last put in that window. It does not hold DevTools.

/// Where DevTools show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Side {
    #[default]
    Bottom,
    Right,
    Window,
}

/// A window's DevTools placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dock {
    pub side: Side,
    /// The page's share of the page area with DevTools below it.
    bottom: f32,
    /// The page's share with DevTools beside it.
    right: f32,
}

impl Default for Dock {
    /// Chrome's first placement: below the page, a third of the area high.
    fn default() -> Self {
        Self { side: Side::Bottom, bottom: 0.66, right: 0.6 }
    }
}

impl Dock {
    /// The page's share on the docked side shown; `None` in a window of their own.
    pub fn ratio(&self) -> Option<f32> {
        match self.side {
            Side::Bottom => Some(self.bottom),
            Side::Right => Some(self.right),
            Side::Window => None,
        }
    }

    /// The divider was dragged: the share is kept for the side it was dragged on.
    pub fn resized(&mut self, ratio: f32) {
        let ratio = ratio.clamp(0., 1.);
        match self.side {
            Side::Bottom => self.bottom = ratio,
            Side::Right => self.right = ratio,
            Side::Window => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_side_keeps_its_own_share() {
        let mut dock = Dock::default();
        assert_eq!(dock.side, Side::Bottom);
        dock.resized(0.4);
        dock.side = Side::Right;
        assert_eq!(dock.ratio(), Some(0.6));
        dock.resized(0.8);
        dock.side = Side::Window;
        assert_eq!(dock.ratio(), None);
        dock.resized(0.1);
        dock.side = Side::Bottom;
        assert_eq!(dock.ratio(), Some(0.4));
        dock.side = Side::Right;
        assert_eq!(dock.ratio(), Some(0.8));
    }

    #[test]
    fn a_share_stays_between_none_and_all() {
        let mut dock = Dock::default();
        dock.resized(1.7);
        assert_eq!(dock.ratio(), Some(1.));
        dock.resized(-0.2);
        assert_eq!(dock.ratio(), Some(0.));
    }
}
