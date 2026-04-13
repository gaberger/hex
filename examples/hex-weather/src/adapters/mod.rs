use crate::domain::{Weather, Temperature, Condition, Forecast, DayForecast};
use crate::ports::{WeatherService, CliParser, CliRequest};

pub struct MockWeatherService;

impl WeatherService for MockWeatherService {
    fn current(&self, city: &str) -> Result<Weather, String> {
        let seed: u64 = city.as_bytes().iter().map(|b| *b as u64).sum();
        let temp = (seed % 35) as f64 - 5.0;
        Ok(Weather {
            city: city.to_string(),
            temperature: Temperature { celsius: temp, fahrenheit: (temp * 9.0 / 5.0) + 32.0 },
            condition: Condition::Sunny,
        })
    }

    fn forecast(&self, city: &str, days: usize) -> Result<Forecast, String> {
        let seed: u64 = city.as_bytes().iter().map(|b| *b as u64).sum();
        let temp = (seed % 35) as f64 - 5.0;
        let day_forecasts = (0..days)
            .map(|_| DayForecast {
                temperature: Temperature { celsius: temp, fahrenheit: (temp * 9.0 / 5.0) + 32.0 },
                condition: Condition::Sunny,
            })
            .collect();
        Ok(Forecast { city: city.to_string(), days: day_forecasts })
    }
}

pub struct EnvCliParser;

impl CliParser for EnvCliParser {
    fn parse(&self) -> Result<CliRequest, String> {
        let args: Vec<String> = std::env::args().collect();
        if args.len() < 2 {
            return Err(String::from("Usage: <city> [--forecast] [--fahrenheit]"));
        }
        let city = &args[1];
        let mut forecast = false;
        let mut fahrenheit = false;

        for arg in &args[2..] {
            match arg.as_str() {
                "--forecast" => forecast = true,
                "--fahrenheit" => fahrenheit = true,
                _ => return Err(String::from("Usage: <city> [--forecast] [--fahrenheit]")),
            }
        }

        Ok(CliRequest { city: city.to_string(), forecast, fahrenheit })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_weather_returns_data() {
        let service = MockWeatherService;
        let result = service.current("London").unwrap();
        assert_eq!(result.city, "London");
    }

    #[test]
    fn test_mock_weather_deterministic() {
        let service = MockWeatherService;
        let first_result = service.current("London").unwrap().temperature.celsius;
        let second_result = service.current("London").unwrap().temperature.celsius;
        assert_eq!(first_result, second_result);
    }

    #[test]
    fn test_mock_forecast_length() {
        let service = MockWeatherService;
        let result = service.forecast("London", 3).unwrap();
        assert_eq!(result.days.len(), 3);
    }
}
