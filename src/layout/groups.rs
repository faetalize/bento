//! Groups of windows: bento's state, which layout engines present however they like.
//!
//! A group is a set of windows and which one of them is active. Order, looks and layout are the
//! engine's. Groups don't care about workspaces or monitors: the windows of a group can be on
//! different ones. A group with fewer than two windows doesn't exist. Windows go by the id that the
//! layout engine knows them by.

use smithay::utils::{Logical, Point, Rectangle};

use super::{Layout, LayoutElement};
use crate::utils::id::IdCounter;

static GROUP_ID_COUNTER: IdCounter = IdCounter::new();

/// An id for a new group.
pub fn next_id() -> u64 {
    GROUP_ID_COUNTER.next()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: u64,
    /// In the order they joined.
    pub windows: Vec<u64>,
    pub active: u64,
}

/// A direction on a workspace's canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Every group, in the order they were made.
#[derive(Debug, Default)]
pub struct Groups {
    groups: Vec<Group>,
}

impl Groups {
    pub fn iter(&self) -> impl Iterator<Item = &Group> {
        self.groups.iter()
    }

    pub fn get(&self, id: u64) -> Option<&Group> {
        self.groups.iter().find(|group| group.id == id)
    }

    /// The group that a window is in.
    pub fn of_window(&self, window: u64) -> Option<&Group> {
        self.groups
            .iter()
            .find(|group| group.windows.contains(&window))
    }

    /// Makes group `id` of `windows`, which leave the groups they're in. `active` is the first
    /// window if it isn't one of them. Returns whether there is a group: it takes two windows.
    pub fn create(&mut self, id: u64, windows: &[u64], active: Option<u64>) -> bool {
        let mut members: Vec<u64> = Vec::with_capacity(windows.len());
        for window in windows {
            if !members.contains(window) {
                members.push(*window);
            }
        }
        if members.len() < 2 {
            return false;
        }

        for window in &members {
            self.leave(*window);
        }
        let active = active
            .filter(|active| members.contains(active))
            .unwrap_or(members[0]);
        self.groups.push(Group {
            id,
            windows: members,
            active,
        });
        true
    }

    /// Adds a window to a group, taking it out of the group it's in. Returns whether the group
    /// exists.
    pub fn add(&mut self, group: u64, window: u64) -> bool {
        let Some(idx) = self.groups.iter().position(|g| g.id == group) else {
            return false;
        };
        if self.groups[idx].windows.contains(&window) {
            return true;
        }

        self.leave(window);
        // Leaving can only dissolve the group the window was in, which isn't this one.
        let group = self.groups.iter_mut().find(|g| g.id == group).unwrap();
        group.windows.push(window);
        true
    }

    /// Takes a window out of a group. Returns whether it was in it.
    pub fn remove(&mut self, group: u64, window: u64) -> bool {
        if !self
            .get(group)
            .is_some_and(|group| group.windows.contains(&window))
        {
            return false;
        }
        self.leave(window);
        true
    }

    /// Makes a window of a group its active one. Returns whether it's in it.
    pub fn activate(&mut self, group: u64, window: u64) -> bool {
        let Some(group) = self.groups.iter_mut().find(|g| g.id == group) else {
            return false;
        };
        if !group.windows.contains(&window) {
            return false;
        }
        group.active = window;
        true
    }

    pub fn dissolve(&mut self, group: u64) {
        self.groups.retain(|g| g.id != group);
    }

    /// Takes a window out of the group it's in, if any. The group is gone when that leaves it with
    /// one window; when the window was the active one, the window that takes its place in the
    /// order becomes active, or the last one if it was last.
    pub fn leave(&mut self, window: u64) {
        let Some(idx) = self
            .groups
            .iter()
            .position(|group| group.windows.contains(&window))
        else {
            return;
        };

        let group = &mut self.groups[idx];
        let pos = group.windows.iter().position(|w| *w == window).unwrap();
        group.windows.remove(pos);
        if group.windows.len() < 2 {
            self.groups.remove(idx);
        } else if group.active == window {
            group.active = group.windows[pos.min(group.windows.len() - 1)];
        }
    }

    /// Takes every window that `is_live` doesn't accept out of its group.
    pub fn retain_windows(&mut self, is_live: impl Fn(u64) -> bool) {
        let gone: Vec<u64> = self
            .groups
            .iter()
            .flat_map(|group| group.windows.iter().copied())
            .filter(|window| !is_live(*window))
            .collect();
        for window in gone {
            self.leave(window);
        }
    }
}

/// The window that's nearest to `from` in `direction`, among `placed`: windows and where the
/// engine put them. Only windows whose centre is on that side of `from`'s count, and the nearest
/// is the one with the smallest gap between the two rects, then the nearest centre.
pub fn nearest(
    from: u64,
    placed: &[(u64, Rectangle<f64, Logical>)],
    direction: Direction,
) -> Option<u64> {
    let (_, rect) = placed.iter().find(|(id, _)| *id == from)?;
    let center = |rect: &Rectangle<f64, Logical>| -> Point<f64, Logical> {
        Point::from((rect.loc.x + rect.size.w / 2., rect.loc.y + rect.size.h / 2.))
    };
    let from_center = center(rect);

    placed
        .iter()
        .filter(|(id, _)| *id != from)
        .filter(|(_, other)| {
            let other = center(other);
            match direction {
                Direction::Left => other.x < from_center.x,
                Direction::Right => other.x > from_center.x,
                Direction::Up => other.y < from_center.y,
                Direction::Down => other.y > from_center.y,
            }
        })
        .map(|(id, other)| {
            let gap_x = (other.loc.x - (rect.loc.x + rect.size.w))
                .max(rect.loc.x - (other.loc.x + other.size.w))
                .max(0.);
            let gap_y = (other.loc.y - (rect.loc.y + rect.size.h))
                .max(rect.loc.y - (other.loc.y + other.size.h))
                .max(0.);
            let other = center(other);
            let dx = other.x - from_center.x;
            let dy = other.y - from_center.y;
            (*id, gap_x * gap_x + gap_y * gap_y, dx * dx + dy * dy)
        })
        .min_by(|a, b| {
            a.1.total_cmp(&b.1)
                .then(a.2.total_cmp(&b.2))
                .then(a.0.cmp(&b.0))
        })
        .map(|(id, _, _)| id)
}

/// A group, with its windows.
pub struct WindowGroup<'a, W> {
    pub id: u64,
    pub windows: Vec<&'a W>,
    pub active: &'a W,
}

impl<W: LayoutElement> Layout<W> {
    /// The groups of windows, with their windows.
    pub fn window_groups(&self) -> Vec<WindowGroup<'_, W>> {
        let window = |engine_id: u64| {
            self.workspaces()
                .flat_map(|(_, _, ws)| ws.scrolling().tiles())
                .find(|tile| tile.engine_id == engine_id)
                .map(|tile| tile.window())
        };

        self.groups
            .iter()
            .filter_map(|group| {
                let windows: Vec<&W> = group.windows.iter().filter_map(|id| window(*id)).collect();
                // A window that's gone leaves its group when the layout is next flushed.
                if windows.len() != group.windows.len() {
                    return None;
                }
                Some(WindowGroup {
                    id: group.id,
                    windows,
                    active: window(group.active)?,
                })
            })
            .collect()
    }

    /// Takes the focused window out of its group.
    pub fn ungroup_window(&mut self) {
        if let Some(window) = self.focused_engine_window() {
            self.groups.leave(window);
        }
        self.flush_engine();
    }

    /// Groups the focused window with the window nearest to it in `direction` on its workspace, by
    /// where the engine put them: it joins that window's group, or the two make a new one. The
    /// focused window is the group's active one.
    pub fn group_window(&mut self, direction: Direction) {
        self.group_window_inner(direction);
        self.flush_engine();
    }

    fn group_window_inner(&mut self, direction: Direction) {
        let Some(focused) = self.focused_engine_window() else {
            return;
        };
        let Some(ws) = self.active_workspace() else {
            return;
        };
        let Some(target) = nearest(focused, &ws.scrolling().placed_windows(), direction) else {
            return;
        };

        match self.groups.of_window(target).map(|group| group.id) {
            Some(group) => {
                self.groups.add(group, focused);
                self.groups.activate(group, focused);
            }
            None => {
                self.groups
                    .create(next_id(), &[target, focused], Some(focused));
            }
        }
    }

    /// Takes the windows that are gone, or that the layout engine doesn't place, out of their
    /// groups: closed windows, floating ones, and ones being dragged.
    pub(super) fn prune_groups(&mut self) {
        if self.groups.iter().next().is_none() {
            return;
        }
        let live: Vec<u64> = self
            .workspaces()
            .flat_map(|(_, _, ws)| ws.scrolling().tiles())
            .map(|tile| tile.engine_id)
            .collect();
        self.groups.retain_windows(|window| live.contains(&window));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rectangle<f64, Logical> {
        Rectangle::new(Point::from((x, y)), (w, h).into())
    }

    fn windows(groups: &Groups) -> Vec<(u64, Vec<u64>, u64)> {
        groups
            .iter()
            .map(|g| (g.id, g.windows.clone(), g.active))
            .collect()
    }

    #[test]
    fn a_group_takes_two_windows() {
        let mut groups = Groups::default();
        assert!(!groups.create(1, &[5], None));
        assert!(!groups.create(1, &[5, 5], None));
        assert!(groups.create(1, &[5, 6, 5], Some(6)));
        assert_eq!(windows(&groups), [(1, vec![5, 6], 6)]);
    }

    #[test]
    fn the_first_window_is_active_unless_another_is_named() {
        let mut groups = Groups::default();
        groups.create(1, &[5, 6], None);
        groups.create(2, &[7, 8], Some(9));
        assert_eq!(windows(&groups), [(1, vec![5, 6], 5), (2, vec![7, 8], 7)]);
    }

    #[test]
    fn grouped_windows_leave_their_groups() {
        let mut groups = Groups::default();
        groups.create(1, &[1, 2, 3], None);
        groups.create(2, &[3, 4], None);
        // 3 left the first group, which has two windows still.
        assert_eq!(windows(&groups), [(1, vec![1, 2], 1), (2, vec![3, 4], 3)]);

        // Both windows of the first group go to a new one: nothing is left of it.
        groups.create(3, &[1, 2, 5], None);
        assert_eq!(
            windows(&groups),
            [(2, vec![3, 4], 3), (3, vec![1, 2, 5], 1)]
        );
    }

    #[test]
    fn a_group_with_one_window_is_gone() {
        let mut groups = Groups::default();
        groups.create(1, &[1, 2], None);
        assert!(groups.remove(1, 2));
        assert!(groups.iter().next().is_none());
        assert!(!groups.remove(1, 1));
    }

    #[test]
    fn adding_a_window_takes_it_from_its_group() {
        let mut groups = Groups::default();
        groups.create(1, &[1, 2], None);
        groups.create(2, &[3, 4, 5], None);
        assert!(groups.add(1, 3));
        assert_eq!(
            windows(&groups),
            [(1, vec![1, 2, 3], 1), (2, vec![4, 5], 4)]
        );

        // Taking the last but one window of a group dissolves it.
        assert!(groups.add(1, 4));
        assert_eq!(windows(&groups), [(1, vec![1, 2, 3, 4], 1)]);

        assert!(groups.add(1, 4));
        assert!(!groups.add(9, 4));
    }

    #[test]
    fn the_next_window_is_active_when_the_active_one_leaves() {
        let mut groups = Groups::default();
        groups.create(1, &[1, 2, 3, 4], Some(2));
        groups.leave(2);
        assert_eq!(windows(&groups), [(1, vec![1, 3, 4], 3)]);

        groups.activate(1, 4);
        groups.leave(4);
        assert_eq!(windows(&groups), [(1, vec![1, 3], 3)]);
    }

    #[test]
    fn only_a_member_can_be_activated() {
        let mut groups = Groups::default();
        groups.create(1, &[1, 2], None);
        assert!(groups.activate(1, 2));
        assert!(!groups.activate(1, 3));
        assert!(!groups.activate(2, 1));
        assert_eq!(windows(&groups), [(1, vec![1, 2], 2)]);
    }

    #[test]
    fn windows_that_are_gone_leave_their_groups() {
        let mut groups = Groups::default();
        groups.create(1, &[1, 2, 3], None);
        groups.create(2, &[4, 5], None);
        groups.retain_windows(|window| window != 1 && window != 5);
        assert_eq!(windows(&groups), [(1, vec![2, 3], 2)]);
    }

    #[test]
    fn dissolving_ungroups_every_window() {
        let mut groups = Groups::default();
        groups.create(1, &[1, 2, 3], None);
        groups.dissolve(1);
        assert!(groups.of_window(1).is_none());
    }

    #[test]
    fn the_nearest_window_is_picked_by_the_gap_then_the_centre() {
        let placed = [
            (1, rect(100., 0., 100., 100.)),
            (2, rect(0., 0., 80., 100.)),
            // Two windows to the left of 2, with the same gap: 3's centre is closer to 2's.
            (3, rect(-100., 0., 80., 40.)),
            (4, rect(-100., 70., 80., 30.)),
            (5, rect(300., 0., 100., 100.)),
        ];
        assert_eq!(nearest(1, &placed, Direction::Left), Some(2));
        assert_eq!(nearest(1, &placed, Direction::Right), Some(5));
        assert_eq!(nearest(2, &placed, Direction::Left), Some(3));
    }

    #[test]
    fn a_window_on_the_other_side_is_not_in_the_direction() {
        let placed = [
            (1, rect(0., 0., 100., 100.)),
            (2, rect(0., 200., 100., 100.)),
        ];
        assert_eq!(nearest(1, &placed, Direction::Down), Some(2));
        assert_eq!(nearest(1, &placed, Direction::Left), None);
        assert_eq!(nearest(3, &placed, Direction::Left), None);
    }
}
