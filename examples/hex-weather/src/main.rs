mod domain;
mod ports;
mod adapters;

use ports::{WeatherService, CliParser};
use adapters::{MockWeatherService, EnvCliParser};

fn main() {
    let cli = EnvCliParser::new();
    let weather_service = MockWeatherService::new();
    let request = match cli.parse() {
        Ok(r) => r,
        Err(e) => { eprintln!("Error: {}", e); std::process::exit(1); }
    };
    
    if request.show_forecast {
        match weather_service.forecast(&request.city, 5) {
            Ok(forecast) => {
                if request.fahrenheit {
                    // Convert temperatures to Fahrenheit
                    let forecast_fahrenheit = forecast.into_iter().map(|mut day| {
                        day.temp = day.temp.to_fahrenheit();
                        day
                    }).collect::<Vec<_>>();
                    for day in forecast_fahrenheit {
                        println!("Date: {}, Temp: {:.1}°F", day.date, day.temp);
                    }
                } else {
                    for day in forecast {
                        println!("Date: {}, Temp: {:.1}°C", day.date, day.temp);
                    }
                }
            }
            Err(e) => { eprintln!("Error: {}", e); std::process::exit(1); }
        }
    } else {
        match weather_service.current(&request.city) {
            Ok(weather) => {
                if request.fahrenheit {
                    println!("Weather in {}: {:.1}°F", weather.city, weather.temp.to_fahrenheit());
                } else {
                    println!("Weather in {}: {:.1}°C", weather.city, weather.temp);
                }
            }
            Err(e) => { eprintln!("Error: {}", e); std::process::exit(1); }
        }
    }
}
