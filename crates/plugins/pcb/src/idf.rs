//! IDF 3.0 board (.emn) and library (.emp) files for MCAD tools.

use crate::{Board, BoardView};

/// `<stem>.emn` and `<stem>.emp`.
pub fn files(_view: &BoardView, _board: &Board) -> Vec<(String, String)> {
    vec![]
}
