//! CLI input adapter — reads stdin lines, returns Commands.
//!
//! Recognised inputs:
//!   <Enter> | "s" | "step"   → Command::Step
//!   "n <N>"                   → Command::StepN(N)
//!   "r" | "reset"             → Command::Reset
//!   "q" | "quit" | EOF        → Command::Quit (None on EOF)

use std::io::{BufRead, Write};

use crate::ports::{Command, IInputPort};

pub struct CliInput<R: BufRead, W: Write> {
    reader: R,
    writer: W,
    buf: String,
}

impl<R: BufRead, W: Write> CliInput<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader, writer, buf: String::new() }
    }
}

impl<R: BufRead, W: Write> IInputPort for CliInput<R, W> {
    fn next_command(&mut self) -> Option<Command> {
        let _ = write!(self.writer, "\n[Enter]=step  n N=stepN  r=reset  q=quit > ");
        let _ = self.writer.flush();
        self.buf.clear();
        let bytes = self.reader.read_line(&mut self.buf).ok()?;
        if bytes == 0 {
            return None; // EOF
        }
        let line = self.buf.trim();
        if line.is_empty() || line == "s" || line == "step" {
            return Some(Command::Step);
        }
        if let Some(rest) = line.strip_prefix("n ") {
            if let Ok(n) = rest.trim().parse::<u32>() {
                return Some(Command::StepN(n));
            }
        }
        if line == "r" || line == "reset" {
            return Some(Command::Reset);
        }
        if line == "q" || line == "quit" {
            return Some(Command::Quit);
        }
        // Unknown — re-prompt.
        let _ = writeln!(self.writer, "  ? unrecognised; try Enter / n 5 / r / q");
        self.next_command()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn run(input: &str) -> Vec<Command> {
        let cursor = Cursor::new(input);
        let reader = BufReader::new(cursor);
        let mut writer: Vec<u8> = Vec::new();
        let mut cli = CliInput::new(reader, &mut writer);
        let mut out = Vec::new();
        while let Some(c) = cli.next_command() {
            out.push(c);
        }
        out
    }

    #[test]
    fn empty_lines_step() {
        assert_eq!(run("\n\n"), vec![Command::Step, Command::Step]);
    }

    #[test]
    fn n_command() {
        assert_eq!(run("n 7\n"), vec![Command::StepN(7)]);
    }

    #[test]
    fn r_resets_q_quits() {
        assert_eq!(run("r\nq\n"), vec![Command::Reset, Command::Quit]);
    }

    #[test]
    fn eof_returns_none() {
        // No trailing newline; read_line on Cursor returns "" for second call.
        assert_eq!(run("s"), vec![Command::Step]);
    }

    #[test]
    fn unknown_reprompts_then_proceeds() {
        // First line is unknown, second line steps.
        assert_eq!(run("garbage\n\n"), vec![Command::Step]);
    }
}
