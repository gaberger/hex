//! hex-fizzbuzz-hermes — minimal hexagonal example built end-to-end by
//! the hex autonomous SOP loop. Demonstrates domain / ports / usecases /
//! adapter / composition-root layering on a trivial domain.

pub mod domain {
    pub fn fizzbuzz(n: u32) -> String {
        match (n % 3, n % 5) {
            (0, 0) => "FizzBuzz".to_string(),
            (0, _) => "Fizz".to_string(),
            (_, 0) => "Buzz".to_string(),
            _ => n.to_string(),
        }
    }
}

pub mod ports {
    pub trait Writer {
        fn write(&mut self, line: &str);
    }
}

pub mod usecases {
    use crate::domain::fizzbuzz;
    use crate::ports::Writer;

    pub fn play<W: Writer>(writer: &mut W, start: u32, end: u32) {
        for n in start..=end {
            writer.write(&fizzbuzz(n));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::domain::fizzbuzz;
    use super::ports::Writer;
    use super::usecases::play;

    #[test]
    fn one_is_one() { assert_eq!(fizzbuzz(1), "1"); }
    #[test]
    fn three_is_fizz() { assert_eq!(fizzbuzz(3), "Fizz"); }
    #[test]
    fn five_is_buzz() { assert_eq!(fizzbuzz(5), "Buzz"); }
    #[test]
    fn fifteen_is_fizzbuzz() { assert_eq!(fizzbuzz(15), "FizzBuzz"); }

    struct Capture(Vec<String>);
    impl Writer for Capture { fn write(&mut self, s: &str) { self.0.push(s.to_string()); } }

    #[test]
    fn play_one_to_five() {
        let mut c = Capture(vec![]);
        play(&mut c, 1, 5);
        assert_eq!(c.0, vec!["1", "2", "Fizz", "4", "Buzz"]);
    }

    #[test]
    fn play_eleven_to_fifteen() {
        let mut c = Capture(vec![]);
        play(&mut c, 11, 15);
        assert_eq!(c.0, vec!["11", "Fizz", "13", "14", "FizzBuzz"]);
    }
}
