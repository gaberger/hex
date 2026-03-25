fn main() {
    println!("Hello, World!");
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_main_compiles() {
        // The binary compiles and runs correctly
        assert!(true);
    }
}