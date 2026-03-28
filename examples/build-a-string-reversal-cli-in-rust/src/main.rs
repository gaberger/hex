use crate::adapters::primary::cli_string_reversal_adapter::CliStringReversalAdapter;

fn main() {
    let input = "Hello, World!";
    let reversed = CliStringReversalAdapter::reverse(input);
    println!("{}", reversed);
}