use adapters::secondary::CliAdapter;

fn main() {
    let adapter = CliAdapter::new();
    adapter.check_palindrome();
}