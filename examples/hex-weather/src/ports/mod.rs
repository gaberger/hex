use crate::domain::{Weather, Forecast};

pub trait WeatherService {
    fn current(&self, city: &str) -> Result<Weather, String>;
    fn forecast(&self, city: &str, days: usize) -> Result<Forecast, String>;
}

pub struct CliRequest {
    pub city: String,
    pub show_forecast: bool,
    pub fahrenheit: bool,
}

pub trait CliParser {
    fn parse(&self) -> Result<CliRequest, String>;
}
