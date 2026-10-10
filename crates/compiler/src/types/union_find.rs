//! Union-find over an arena (`Type/UnionFind.hs`). Upstream keeps each
//! point in an `IORef`; here points are indices into one store, which the
//! type checker owns, so every operation takes it.
//!
//! Points join by weight, the heavier one becoming the root and the
//! first given on a tie, and finding a root shortens the path to it, as
//! upstream does.

/// A point: an index into its [`UnionFind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point(u32);

impl Point {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Debug)]
enum Info<T> {
    /// A root: how many points it stands for, and their value.
    Root {
        weight: u32,
        value: T,
    },
    Link(Point),
}

/// Points, each linked to another or the root of a set with a value.
#[derive(Clone, Debug)]
pub struct UnionFind<T> {
    points: Vec<Info<T>>,
}

impl<T> Default for UnionFind<T> {
    fn default() -> UnionFind<T> {
        UnionFind { points: Vec::new() }
    }
}

impl<T> UnionFind<T> {
    /// `fresh`: a point in a set of its own.
    pub fn fresh(&mut self, value: T) -> Point {
        let point = Point(u32::try_from(self.points.len()).expect("fewer than 2^32 points"));
        self.points.push(Info::Root { weight: 1, value });
        point
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// `repr`: the root of `point`'s set, each point passed on the way
    /// linked straight to it.
    fn repr(&mut self, point: Point) -> Point {
        let mut root = point;
        while let Info::Link(next) = self.points[root.index()] {
            root = next;
        }
        let mut at = point;
        while let Info::Link(next) = self.points[at.index()] {
            if next != root {
                self.points[at.index()] = Info::Link(root);
            }
            at = next;
        }
        root
    }

    /// The value of `point`'s set.
    pub fn get(&mut self, point: Point) -> &T {
        let root = self.repr(point);
        match &self.points[root.index()] {
            Info::Root { value, .. } => value,
            Info::Link(_) => unreachable!("a root links nowhere"),
        }
    }

    /// The value of `point`'s set, to change.
    pub fn get_mut(&mut self, point: Point) -> &mut T {
        let root = self.repr(point);
        match &mut self.points[root.index()] {
            Info::Root { value, .. } => value,
            Info::Link(_) => unreachable!("a root links nowhere"),
        }
    }

    pub fn set(&mut self, point: Point, value: T) {
        *self.get_mut(point) = value;
    }

    /// `union`: one set of `a`'s and `b`'s, with `value`.
    pub fn union(&mut self, a: Point, b: Point, value: T) {
        let (a, b) = (self.repr(a), self.repr(b));
        if a == b {
            self.set(a, value);
            return;
        }
        let weight = |info: &Info<T>| match info {
            Info::Root { weight, .. } => *weight,
            Info::Link(_) => unreachable!("a root links nowhere"),
        };
        let (wa, wb) = (
            weight(&self.points[a.index()]),
            weight(&self.points[b.index()]),
        );
        let (root, child) = if wa >= wb { (a, b) } else { (b, a) };
        self.points[child.index()] = Info::Link(root);
        self.points[root.index()] = Info::Root {
            weight: wa + wb,
            value,
        };
    }

    /// `equivalent`: whether `a` and `b` are in one set.
    pub fn equivalent(&mut self, a: Point, b: Point) -> bool {
        self.repr(a) == self.repr(b)
    }

    /// `redundant`: whether `point` links to another, so a set holds it
    /// under another root.
    pub fn redundant(&self, point: Point) -> bool {
        matches!(self.points[point.index()], Info::Link(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_join_by_weight_and_share_a_value() {
        let mut uf = UnionFind::default();
        let (a, b, c) = (uf.fresh('a'), uf.fresh('b'), uf.fresh('c'));
        uf.union(a, b, 'x');
        assert!(uf.equivalent(a, b));
        assert!(!uf.equivalent(a, c));
        // A tie keeps the first as the root.
        assert!(!uf.redundant(a));
        assert!(uf.redundant(b));
        // The heavier set's root stays the root.
        uf.union(c, b, 'y');
        assert!(uf.redundant(c));
        assert_eq!((*uf.get(a), *uf.get(b), *uf.get(c)), ('y', 'y', 'y'));
        uf.set(c, 'z');
        assert_eq!(*uf.get(a), 'z');
    }

    #[test]
    fn a_long_chain_shortens() {
        let mut uf = UnionFind::default();
        let points: Vec<Point> = (0..1000).map(|i| uf.fresh(i)).collect();
        for pair in points.windows(2) {
            // Each new point is lighter than the set it joins.
            uf.union(pair[0], pair[1], 0);
        }
        assert!(uf.equivalent(points[0], points[999]));
        assert_eq!(*uf.get(points[999]), 0);
    }
}
