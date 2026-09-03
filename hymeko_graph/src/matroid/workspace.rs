//! The caller-owned union-find scratch shared by both matroids. Path-halving `find`, union by rank, and
//! a generation-stamped lazy reset (no O(N) clear between queries). It stores **no** canonical topology —
//! only DSU bookkeeping over an abstract dense coordinate space (nodes for rank-2; nodes ⊕ hyperarcs for
//! Levi). The `begin` / `union` primitives are `pub(in crate::matroid)` so `rank2` and `levi` share this
//! one implementation rather than each carrying its own.

use super::MatroidError;

/// Caller-owned reusable DSU scratch, shared by the rank-2 and Levi matroids.
#[derive(Default)]
pub struct GraphicMatroidWorkspace {
    parent: Vec<u32>,
    rank: Vec<u32>,
    generation: Vec<u32>,
    cur_gen: u32,
    /// General-purpose scratch (e.g. sorted edge indices for a weighted basis).
    scratch: Vec<u32>,
}

impl GraphicMatroidWorkspace {
    /// A fresh, empty workspace. Grows lazily on first use to fit the coordinate universe.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Retained scratch bytes (for memory accounting; not topology).
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        (self.parent.capacity()
            + self.rank.capacity()
            + self.generation.capacity()
            + self.scratch.capacity())
            * std::mem::size_of::<u32>()
    }

    /// Begin a query over an `n`-coordinate universe: size the arrays (never shrink), bump the generation
    /// (so every cell lazily re-initialises on first touch), and clear the scratch.
    ///
    /// # Errors
    /// [`MatroidError::Overflow`] if `n` exceeds `u32`.
    pub(in crate::matroid) fn begin(&mut self, n: usize) -> Result<(), MatroidError> {
        if u32::try_from(n).is_err() {
            return Err(MatroidError::Overflow);
        }
        if self.parent.len() < n {
            self.parent.resize(n, 0);
            self.rank.resize(n, 0);
            self.generation.resize(n, 0);
        }
        self.cur_gen = self.cur_gen.wrapping_add(1);
        if self.cur_gen == 0 {
            self.generation.iter_mut().for_each(|g| *g = 0);
            self.cur_gen = 1;
        }
        self.scratch.clear();
        Ok(())
    }

    #[inline]
    fn touch(&mut self, x: u32) {
        let i = x as usize;
        if self.generation[i] != self.cur_gen {
            self.generation[i] = self.cur_gen;
            self.parent[i] = x;
            self.rank[i] = 0;
        }
    }

    fn find(&mut self, mut x: u32) -> u32 {
        self.touch(x);
        while self.parent[x as usize] != x {
            let p = self.parent[x as usize];
            self.touch(p);
            let gp = self.parent[p as usize];
            self.touch(gp);
            self.parent[x as usize] = gp; // path halving
            x = gp;
        }
        x
    }

    /// Union `a` and `b`; returns `true` iff they were in different sets (i.e. this edge joins the forest).
    pub(in crate::matroid) fn union(&mut self, a: u32, b: u32) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        let (ra, rb) = (ra as usize, rb as usize);
        match self.rank[ra].cmp(&self.rank[rb]) {
            std::cmp::Ordering::Less => self.parent[ra] = rb as u32,
            std::cmp::Ordering::Greater => self.parent[rb] = ra as u32,
            std::cmp::Ordering::Equal => {
                self.parent[rb] = ra as u32;
                self.rank[ra] += 1;
            }
        }
        true
    }
}
