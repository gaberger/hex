use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage: {} <temp> <from_unit> <to_unit>", args[0]);
        std::process::exit(1);
    }
    let temp = args[1].parse().expect("Invalid temperature");
    let from = args[2].to_lowercase();
    let to = args[3].to_lowercase();

    let result = convert_temperature(temp, &from, &to);
    println!("Result: {}", result);
}

fn convert_temperature(temp: f64, from: &str, to: &str) -> f64 {
    match (from, to) {
        ("c", "f") => temp * 9.0/5.0 + 32.0,
        ("f", "c") => (temp - 32.0) * 5.0/9.0,
        _ => temp,
    }
}