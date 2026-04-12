use std::env;
use std::fmt;

#[derive(Debug)]
struct WeatherData {
    city: String,
    temperature_c: f64,
    humidity: u8,
    description: String,
    wind_speed_kmh: f64,
}

impl fmt::Display for WeatherData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Weather in {}: {:.1}°C, Humidity: {}%, Description: {}, Wind Speed: {:.1} km/h",
            self.city, self.temperature_c, self.humidity, self.description, self.wind_speed_kmh
        )
    }
}

#[derive(Debug)]
struct ForecastDay {
    date: String,
    high_c: f64,
    low_c: f64,
    description: String,
}

#[derive(Debug)]
struct Forecast {
    city: String,
    days: Vec<ForecastDay>,
}

impl fmt::Display for Forecast {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Forecast for {}", self.city)?;
        for day in &self.days {
            writeln!(
                f,
                "{}: High {:.1}°C, Low {:.1}°C, Description: {}",
                day.date, day.high_c, day.low_c, day.description
            )?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct CliArgs {
    city: String,
    forecast: bool,
    units: char,
}

fn parse_args() -> Result<CliArgs, String> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 || args.len() > 4 {
        return Err(String::from("Usage: weather_cli <city> [--forecast] [-u f|c]"));
    }

    let mut city = String::new();
    let mut forecast = false;
    let mut units = 'c';

    for arg in &args[1..] {
        match arg.as_str() {
            "--forecast" => forecast = true,
            "-u" if args.len() > 2 && (args[args.iter().position(|x| x == "-u").unwrap() + 1].chars().next().unwrap() == 'f' || args[args.iter().position(|x| x == "-u").unwrap() + 1].chars().next().unwrap() == 'c') => {
                units = args[args.iter().position(|x| x == "-u").unwrap() + 1].chars().next().unwrap();
            },
            _ if city.is_empty() => city = arg.to_string(),
            _ => return Err(String::from("Invalid argument")),
        }
    }

    if city.is_empty() {
        return Err(String::from("City is required"));
    }

    Ok(CliArgs { city, forecast, units })
}

fn get_weather(city: &str) -> WeatherData {
    let hash = city.chars().next().unwrap() as u32;
    WeatherData {
        city: city.to_string(),
        temperature_c: ((hash % 5) as f64 - 2.5),
        humidity: (hash % 100) as u8,
        description: match hash % 4 {
            0 => "Sunny".to_string(),
            1 => "Cloudy".to_string(),
            2 => "Rainy".to_string(),
            _ => "Snowy".to_string(),
        },
        wind_speed_kmh: ((hash % 20) as f64 * 2.5),
    }
}

fn get_forecast(city: &str, days: usize) -> Forecast {
    let mut forecast_days = Vec::new();
    for day in 1..=days {
        let hash = (city.chars().next().unwrap() as u32 + day as u32);
        forecast_days.push(ForecastDay {
            date: format!("Day {}", day),
            high_c: ((hash % 5) as f64 - 2.5) + 10.0,
            low_c: ((hash % 5) as f64 - 2.5) - 5.0,
            description: match hash % 4 {
                0 => "Sunny".to_string(),
                1 => "Cloudy".to_string(),
                2 => "Rainy".to_string(),
                _ => "Snowy".to_string(),
            },
        });
    }
    Forecast {
        city: city.to_string(),
        days: forecast_days,
    }
}

fn main() {
    match parse_args() {
        Ok(args) => {
            if args.forecast {
                let mut forecast = get_forecast(&args.city, 5);
                if args.units == 'f' {
                    for day in &mut forecast.days {
                        day.high_c = (day.high_c * 9.0 / 5.0) + 32.0;
                        day.low_c = (day.low_c * 9.0 / 5.0) + 32.0;
                    }
                }
                println!("{}", forecast);
            } else {
                let mut weather = get_weather(&args.city);
                if args.units == 'f' {
                    weather.temperature_c = (weather.temperature_c * 9.0 / 5.0) + 32.0;
                }
                println!("{}", weather);
            }
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}
