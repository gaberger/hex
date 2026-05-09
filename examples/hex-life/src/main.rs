//! hex-life CLI — interactive Conway's Life on a hex grid.
//!
//! Composition root only. Wires + runs. Press Enter to step. Try `n 20`
//! to advance 20 generations, `r` to reset, `q` to quit.

use hex_life::composition_root::wire;

fn main() {
    println!("hex-life — Conway on a hexagonal grid (B2/S34)");
    println!("rules: dead cell with exactly 2 live neighbours is born;");
    println!("       live cell with 3 or 4 live neighbours survives;");
    println!("       6 neighbours per cell (axial coordinates).");

    let mut app = wire();
    app.game.run(&mut app.input, &mut app.display);
    println!("\n[hex-life] generation {} on exit · {} alive",
             app.game.generation(),
             app.game.current().alive_count());
}
