use super::domain::{Forecast, TemperatureUnit, WeatherData};
use super::ports::{CliParser, WeatherService};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use std::env;

pub struct MockWeatherService;

impl WeatherService for MockWeatherService {
    fn get_weather(&self, city: &str) -> WeatherData {
        let seed: u64 = city.as_bytes().iter().sum();
        let mut rng = StdRng::seed_from_u64(seed);

        let temperatures = [20, 22, 19, 25, 30, 18, 21];
        let temp = *temperatures.choose(&mut rng).unwrap();

        WeatherData {
            temperature: temp,
            description: String::from("Sunny"),
        }
    }

    fn get_forecast(&self, city: &str) -> Forecast {
        let seed: u64 = city.as_bytes().iter().sum();
        let mut rng = StdRng::seed_from_u64(seed);

        let temperatures = [20, 22, 19, 25, 30, 18, 21];
        let temps: Vec<i32> = (0..7).map(|_| *temperatures.choose(&mut rng).unwrap()).collect();

        Forecast {
            temperatures: temps,
        }
    }
}

pub struct EnvCliParser;

impl CliParser for EnvCliParser {
    fn parse_args(&self) -> (String, bool, TemperatureUnit) {
        let args: Vec<String> = env::args().collect();
        if args.len() < 2 {
            panic!("City name is required.");
        }

        let city = args[1].clone();
        let mut forecast = false;
        let mut unit = TemperatureUnit::Celsius;

        for arg in &args[2..] {
            match arg.as_str() {
                "--forecast" => forecast = true,
                "--fahrenheit" => unit = TemperatureUnit::Fahrenheit,
                _ => panic!("Unknown argument: {}", arg),
            }
        }

        (city, forecast, unit)
    }
}
