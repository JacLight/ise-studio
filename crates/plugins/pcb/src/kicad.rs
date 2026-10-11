//! KiCad interop: `.kicad_pcb` in and out.

use crate::{Board, BoardView};
use de_core::{Command, CommandError, ErrorCode, Tx, Value, schemars};
use schemars::JsonSchema;
use serde::Deserialize;

/// The board as a KiCad 8 `.kicad_pcb` file.
pub fn export(_view: &BoardView, _board: &Board) -> String {
    String::new()
}

/// `pcb.board.import_kicad`: a `.kicad_pcb` file's text becomes a Forma board.
#[derive(Deserialize, JsonSchema)]
pub struct ImportKicad {
    /// The whole `.kicad_pcb` file.
    pub text: String,
    /// Board name (default: the file's title, else "Imported board").
    pub name: Option<String>,
    /// Where the 3D board sits in the project (mm).
    pub at: Option<[f64; 3]>,
}

impl Command for ImportKicad {
    const NAME: &'static str = "pcb.board.import_kicad";

    fn apply(self, _tx: &mut Tx) -> Result<Value, CommandError> {
        Err(CommandError::new(ErrorCode::InvalidParameters, "not built yet"))
    }
}
