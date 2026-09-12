#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Conway's Game of Life on a fixed rectangle of cells.
//!
//! A [`Grid`] holds a rectangle of cells. Each cell is alive or dead.
//! [`Grid::step`] moves every cell forward one generation at the same moment,
//! by the standard B3/S23 rule:
//!
//! * A dead cell with exactly 3 live neighbours becomes alive.
//! * A live cell with 2 or 3 live neighbours stays alive.
//! * Every other cell is dead.
//!
//! The rectangle has a hard edge. There is no wrapping. A coordinate outside
//! the rectangle always reads as dead, so an edge cell never sees the far side.
//!
//! The crate has zero dependencies, no threads, no `unsafe`, and no file
//! format.
//!
//! ```
//! use game_life_rs::Grid;
//!
//! let mut g = Grid::new(7, 5);
//! g.set(2, 1, true);
//! g.set(3, 1, true);
//! g.set(4, 1, true);
//! g.step();
//! assert!(g.get(3, 0) && g.get(3, 1) && g.get(3, 2));
//! assert!(!g.get(2, 1) && !g.get(4, 1));
//! ```
//!
//! # Invariants
//!
//! 1. The shape never changes. Both buffers always hold `stride * (height + 2)`
//!    bytes, where `stride == width + 2`.
//! 2. Every coordinate inside the rectangle maps to exactly one buffer address,
//!    and that address is inside the buffer.
//! 3. Outside the rectangle is dead, and can never become alive.
//! 4. The padding ring around the rectangle stays dead. Nothing writes to it.
//! 5. [`Grid::step`] writes every interior cell. No cell keeps a stale value.
//! 6. [`Grid::step`] is pure. It reads no clock, no random number, and no
//!    global state, so the same grid always gives the same answer.
//! 7. [`Grid::step`] never changes the width or the height.
//! 8. A grid with no live cells stays empty forever.
//! 9. [`Grid::step`] never panics and never allocates, for any grid built
//!    through the public API, in any build. This includes a zero-size grid.
//!
//! # About `debug_assert`
//!
//! Invariant 9 says `step` never panics. A `debug_assert` is a panic in a debug
//! build, so the two statements need one rule to hold them together: a
//! `debug_assert` in this crate fires when the *library* has a bug. It never
//! fires because of caller input. The ring check (invariant 4) is such an
//! assertion. A `debug_assert` does nothing under `cargo test --release`, so
//! run the test suite in both profiles.

use std::fmt;

/// The reason a grid could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridError {
    /// The grid size does not fit in memory addresses.
    SizeOverflow,
}

impl fmt::Display for GridError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GridError::SizeOverflow => {
                f.write_str("the grid size does not fit in memory addresses")
            }
        }
    }
}

impl std::error::Error for GridError {}

/// A rectangle of cells that moves forward one generation at a time.
///
/// # Equality
///
/// Two grids are equal when they have the same width, the same height, and the
/// same front buffer. The back buffer is not compared, because it is scratch
/// space that holds an old picture. The generation counter is not compared
/// either, on purpose: a period test clones a grid at generation 0 and compares
/// it against generation 2.
#[derive(Clone)]
pub struct Grid {
    width: usize,
    height: usize,
    /// Always `width + 2`. One dead column on each side.
    stride: usize,
    generation: u64,
    /// The picture the caller can see. Length is `stride * (height + 2)`.
    front: Box<[u8]>,
    /// Scratch space for the next generation. Same length as `front`.
    back: Box<[u8]>,
}

impl Grid {
    /// Builds a `width` by `height` grid with every cell dead.
    ///
    /// # Panics
    ///
    /// Panics when the padded size overflows the address space. The message
    /// names the width and the height. Use [`Grid::try_new`] to get a
    /// [`GridError`] instead.
    ///
    /// # One honest limit
    ///
    /// `Grid::new(3_000_000, 3_000_000)` does not overflow. It asks for about
    /// 9 terabytes. When memory runs out, Rust stops the whole process. It does
    /// not unwind, so no panic message ever prints, and [`Grid::try_new`]
    /// cannot catch it either. This library does not handle that case.
    pub fn new(width: usize, height: usize) -> Grid {
        match Grid::try_new(width, height) {
            Ok(grid) => grid,
            Err(_) => panic!("Grid::new({width}, {height}) overflows the address space"),
        }
    }

    /// Builds a `width` by `height` grid, or reports why it could not.
    ///
    /// Never panics. The arithmetic is checked on the *padded* size, so a
    /// 32-bit target rejects sizes that a `width * height` check would let
    /// through.
    ///
    /// See [`Grid::new`] for the one limit this function cannot report.
    pub fn try_new(width: usize, height: usize) -> Result<Grid, GridError> {
        let stride = width.checked_add(2).ok_or(GridError::SizeOverflow)?;
        let rows = height.checked_add(2).ok_or(GridError::SizeOverflow)?;
        let len = stride.checked_mul(rows).ok_or(GridError::SizeOverflow)?;

        Ok(Grid {
            width,
            height,
            stride,
            generation: 0,
            front: vec![0u8; len].into_boxed_slice(),
            back: vec![0u8; len].into_boxed_slice(),
        })
    }

    /// The number of columns. Never changes after the grid is built.
    pub fn width(&self) -> usize {
        self.width
    }

    /// The number of rows. Never changes after the grid is built.
    pub fn height(&self) -> usize {
        self.height
    }

    /// How many times [`Grid::step`] has run. Saturates at `u64::MAX`.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Reads a cell. Never panics.
    ///
    /// A coordinate outside the rectangle is dead and returns `false`. That is
    /// the dead-edge rule.
    pub fn get(&self, x: usize, y: usize) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        self.front[self.index(x, y)] == 1
    }

    /// Reads a cell, and says whether the coordinate was inside the grid.
    ///
    /// Returns `None` for a coordinate outside the rectangle. Never panics.
    pub fn try_get(&self, x: usize, y: usize) -> Option<bool> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(self.front[self.index(x, y)] == 1)
    }

    /// Writes a cell in the visible picture.
    ///
    /// # Panics
    ///
    /// Panics when the coordinate is outside the rectangle, because there is
    /// nowhere to put the cell. A quiet drop would hide a caller bug for
    /// months. The message shows the coordinate and the grid size, for example
    /// `set(7, 2) is outside a 5x9 grid`. Use [`Grid::try_set`] to get a
    /// `bool` instead.
    pub fn set(&mut self, x: usize, y: usize, alive: bool) {
        if x >= self.width || y >= self.height {
            panic!(
                "set({}, {}) is outside a {}x{} grid",
                x, y, self.width, self.height
            );
        }
        let i = self.index(x, y);
        self.front[i] = u8::from(alive);
    }

    /// Writes a cell, and reports whether the write happened.
    ///
    /// Returns `true` when the cell was written, and `false` when the
    /// coordinate was outside the rectangle. Never panics.
    pub fn try_set(&mut self, x: usize, y: usize, alive: bool) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let i = self.index(x, y);
        self.front[i] = u8::from(alive);
        true
    }

    /// Moves every cell forward one generation, all at the same moment.
    ///
    /// Reads the whole front buffer, writes the whole back buffer, then swaps
    /// the two and adds one to the generation counter. Never panics, and never
    /// allocates.
    pub fn step(&mut self) {
        let s = self.stride;
        for y in 0..self.height {
            for x in 0..self.width {
                let i = self.index(x, y);
                let n = self.front[i - s - 1]
                    + self.front[i - s]
                    + self.front[i - s + 1]
                    + self.front[i - 1]
                    + self.front[i + 1]
                    + self.front[i + s - 1]
                    + self.front[i + s]
                    + self.front[i + s + 1];
                // Rule 1: every interior cell is written, every step. The
                // `else` branch is what stops a stale picture surviving.
                self.back[i] = if n == 3 || (n == 2 && self.front[i] == 1) {
                    1
                } else {
                    0
                };
            }
        }
        std::mem::swap(&mut self.front, &mut self.back);
        self.generation = self.generation.saturating_add(1);
        debug_assert!(
            self.ring_is_dead(),
            "library bug: the padding ring must stay dead"
        );
    }

    /// Turns a public coordinate into a buffer address.
    ///
    /// This formula appears once in the whole crate. Every read and every write
    /// goes through it. The `+ 1` on each axis skips the dead padding ring.
    fn index(&self, x: usize, y: usize) -> usize {
        (y + 1) * self.stride + (x + 1)
    }

    /// Checks invariant 4: the padding ring is dead in both buffers.
    ///
    /// Used only by a `debug_assert` inside [`Grid::step`].
    fn ring_is_dead(&self) -> bool {
        let s = self.stride;
        let rows = self.height + 2;
        for buf in [&self.front, &self.back] {
            for x in 0..s {
                if buf[x] != 0 || buf[(rows - 1) * s + x] != 0 {
                    return false;
                }
            }
            for y in 0..rows {
                if buf[y * s] != 0 || buf[y * s + s - 1] != 0 {
                    return false;
                }
            }
        }
        true
    }
}

impl PartialEq for Grid {
    /// Same width, same height, same visible picture. The back buffer and the
    /// generation counter are not compared. See the type documentation.
    fn eq(&self, other: &Grid) -> bool {
        self.width == other.width && self.height == other.height && self.front == other.front
    }
}

impl Eq for Grid {}

impl fmt::Debug for Grid {
    /// Prints the picture, not the raw bytes, so a failing `assert_eq!` shows
    /// two pictures side by side. `#` is alive and `.` is dead.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "\nGrid {}x{} generation {}",
            self.width, self.height, self.generation
        )?;
        for y in 0..self.height {
            f.write_str("  ")?;
            for x in 0..self.width {
                f.write_str(if self.get(x, y) { "#" } else { "." })?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}
