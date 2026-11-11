//! Grid and pixel geometry. All sizes are physical pixels.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSize {
    pub w: u32,
    pub h: u32,
}

/// row 0 is the top visible row, whatever the scroll position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellPoint {
    pub col: u16,
    pub row: u16,
}

/// Which half of a cell the pointer is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// How many whole cells fit in `rect`. Never below 1 by 1, so a minimised window or a sliver
/// during a resize still has a grid to keep its text in.
pub fn fit(rect: Rect, cell: CellSize) -> GridSize {
    // A zero cell size would divide by zero; one pixel is the smallest real cell.
    let cols = rect.w / cell.w.max(1);
    let rows = rect.h / cell.h.max(1);
    GridSize { cols: at_least_one(cols), rows: at_least_one(rows) }
}

fn at_least_one(n: u32) -> u16 {
    n.clamp(1, u32::from(u16::MAX)) as u16
}

/// The cell under the pixel `(x, y)`, clamped into the grid, so a drag past an edge still names
/// the nearest cell.
pub fn cell_at(rect: Rect, cell: CellSize, grid: GridSize, x: f64, y: f64) -> (CellPoint, Side) {
    let w = f64::from(cell.w.max(1));
    let h = f64::from(cell.h.max(1));
    let dx = x - f64::from(rect.x);
    let dy = y - f64::from(rect.y);
    let col = (dx / w).floor();
    let row = (dy / h).floor();
    let last_col = f64::from(grid.cols.saturating_sub(1));
    let last_row = f64::from(grid.rows.saturating_sub(1));

    // Past the left or right edge, the side is the edge's, so a selection dragged out of the
    // grid takes in the whole of the last cell.
    let side = if col < 0.0 {
        Side::Left
    } else if col > last_col {
        Side::Right
    } else if dx - col * w < w / 2.0 {
        Side::Left
    } else {
        Side::Right
    };
    let point = CellPoint {
        col: col.clamp(0.0, last_col) as u16,
        row: row.clamp(0.0, last_row) as u16,
    };
    (point, side)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: CellSize = CellSize { w: 10, h: 20 };
    const GRID: GridSize = GridSize { cols: 80, rows: 24 };

    #[test]
    fn halves_of_a_cell_give_their_side() {
        let rect = Rect { x: 0, y: 0, w: 800, h: 480 };
        assert_eq!(cell_at(rect, CELL, GRID, 32.0, 45.0), (CellPoint { col: 3, row: 2 }, Side::Left));
        assert_eq!(cell_at(rect, CELL, GRID, 37.0, 45.0), (CellPoint { col: 3, row: 2 }, Side::Right));
    }

    #[test]
    fn points_outside_the_rect_clamp_into_the_grid() {
        let rect = Rect { x: 0, y: 0, w: 800, h: 480 };
        assert_eq!(cell_at(rect, CELL, GRID, -5.0, -5.0), (CellPoint { col: 0, row: 0 }, Side::Left));
        assert_eq!(
            cell_at(rect, CELL, GRID, 5000.0, 5000.0),
            (CellPoint { col: 79, row: 23 }, Side::Right)
        );
    }

    #[test]
    fn the_rect_offset_is_subtracted() {
        let rect = Rect { x: 100, y: 200, w: 800, h: 480 };
        assert_eq!(cell_at(rect, CELL, GRID, 101.0, 201.0), (CellPoint { col: 0, row: 0 }, Side::Left));
        assert_eq!(cell_at(rect, CELL, GRID, 128.0, 241.0), (CellPoint { col: 2, row: 2 }, Side::Right));
    }
}
