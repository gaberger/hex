//! game_loop — composes domain::tick with display + input ports.
//!
//! No I/O of its own. Driven by the IInputPort, renders via IDisplayPort,
//! advances state via domain::tick. Pure logic; deterministic given the
//! same initial grid + same input sequence.

use crate::domain::{tick, Grid};
use crate::ports::{Command, IDisplayPort, IInputPort};

pub struct GameLoop {
    initial: Grid,
    grid: Grid,
    generation: u64,
}

impl GameLoop {
    pub fn new(initial: Grid) -> Self {
        Self {
            grid: initial.clone(),
            initial,
            generation: 0,
        }
    }

    pub fn current(&self) -> &Grid {
        &self.grid
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Run until input port returns None or operator issues Quit.
    pub fn run(&mut self, input: &mut dyn IInputPort, display: &mut dyn IDisplayPort) {
        display.render(self.generation, &self.grid);
        while let Some(cmd) = input.next_command() {
            match cmd {
                Command::Step => {
                    self.grid = tick(&self.grid);
                    self.generation += 1;
                    display.render(self.generation, &self.grid);
                }
                Command::StepN(n) => {
                    for _ in 0..n {
                        self.grid = tick(&self.grid);
                        self.generation += 1;
                    }
                    display.render(self.generation, &self.grid);
                }
                Command::Reset => {
                    self.grid = self.initial.clone();
                    self.generation = 0;
                    display.render(self.generation, &self.grid);
                }
                Command::Quit => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Coord;

    struct ScriptedInput(Vec<Command>);
    impl IInputPort for ScriptedInput {
        fn next_command(&mut self) -> Option<Command> {
            if self.0.is_empty() { None } else { Some(self.0.remove(0)) }
        }
    }

    struct CountingDisplay {
        renders: u32,
        last_alive: usize,
    }
    impl IDisplayPort for CountingDisplay {
        fn render(&mut self, _gen: u64, grid: &Grid) {
            self.renders += 1;
            self.last_alive = grid.alive_count();
        }
    }

    #[test]
    fn loop_steps_then_quits() {
        let initial = Grid::from_alive([Coord::new(0, 0), Coord::new(1, 0)]);
        let mut input = ScriptedInput(vec![Command::Step, Command::Step, Command::Quit]);
        let mut display = CountingDisplay { renders: 0, last_alive: 0 };
        let mut g = GameLoop::new(initial);
        g.run(&mut input, &mut display);
        assert_eq!(g.generation(), 2);
        // Initial render + 2 steps = 3 renders.
        assert_eq!(display.renders, 3);
    }

    #[test]
    fn reset_restores_initial() {
        let initial = Grid::from_alive([Coord::new(0, 0), Coord::new(1, 0)]);
        let initial_count = initial.alive_count();
        let mut input = ScriptedInput(vec![
            Command::StepN(5),
            Command::Reset,
            Command::Quit,
        ]);
        let mut display = CountingDisplay { renders: 0, last_alive: 0 };
        let mut g = GameLoop::new(initial);
        g.run(&mut input, &mut display);
        assert_eq!(g.generation(), 0);
        assert_eq!(display.last_alive, initial_count);
    }
}
