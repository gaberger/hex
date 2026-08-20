//! Primary adapter + composition root for hex-fizzbuzz-hermes.
//! Parses CLI args, wires a stdout Writer, dispatches to the usecase.

use hex_fizzbuzz_hermes::ports::Writer;
use hex_fizzbuzz_hermes::usecases::play;

struct Stdout;
impl Writer for Stdout {
    fn write(&mut self, line: &str) { println!("{}", line); }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let start: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let end: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(15);
    if end < start {
        eprintln!("usage: fizzbuzz <start> <end>  (end must be >= start)");
        std::process::exit(2);
    }
    let mut out = Stdout;
    play(&mut out, start, end);
}
