use fibonacci_cli_adapter::cli::Cli;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n = args[1].parse().expect("Please provide a number");
    let sequence = Cli::compute_fibonacci(n);
    println!("{:?}", sequence);
}