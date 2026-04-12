mod domain;
mod ports;
mod adapters;

use ports::{WeatherService, CliParser, CliRequest};
use adapters::{MockWeatherService, EnvCliParser};

fn main() {
    let cli: Box<dyn CliParser> = Box::new(EnvCliParser);
    let weather: Box<dyn WeatherService> = Box::new(MockWeatherService);

    let request = match cli.parse() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    if request.show_forecast {
        match weather.forecast(&request.city, 5) {
            Ok(forecast) => {
                for (day, temp_c) in forecast.iter().enumerate() {
                    let temp_f = celsius_to_fahrenheit(*temp_c);
                    println!("Day {}: {:.1}°C / {:.1}°F", day + 1, temp_c, temp_f);
                }
            }
            Err(e) => eprintln!("Error fetching forecast: {}", e),
        }
    } else {
        match weather.current(&request.city) {
            Ok(temp_c) => {
                let temp_f = celsius_to_fahrenheit(temp_c);
                println!("Current temperature in {}: {:.1}°C / {:.1}°F", request.city, temp_c, temp_f);
            }
            Err(e) => eprintln!("Error fetching current weather: {}", e),
        }
    }
}

fn celsius_to_fahrenheit(celsius: f64) -> f64 {
    celsius * 9.0 / 5.0 + 32.0
}
