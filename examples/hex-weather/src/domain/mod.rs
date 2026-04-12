use std::fmt;

pub struct Temperature {
    pub celsius: f64,
}

impl Temperature {
    pub fn to_fahrenheit(&self) -> f64 {
        self.celsius * 9.0 / 5.0 + 32.0
    }
}

impl fmt::Display for Temperature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1}°C", self.celsius)
    }
}

pub enum Condition {
    Sunny,
    Cloudy,
    Rainy,
    Snowy,
    Windy,
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Condition::Sunny => "Sunny",
            Condition::Cloudy => "Cloudy",
            Condition::Rainy => "Rainy",
            Condition::Snowy => "Snowy",
            Condition::Windy => "Windy",
        })
    }
}

pub struct Weather {
    pub city: String,
    pub temp: Temperature,
    pub humidity: u8,
    pub condition: Condition,
}

impl fmt::Display for Weather {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Weather in {}: {}°C ({}), Humidity: {}%, {}", self.city, self.temp.celsius, self.temp.to_fahrenheit(), self.humidity, self.condition)
    }
}

pub struct DayForecast {
    pub date: String,
    pub high: Temperature,
    pub low: Temperature,
    pub condition: Condition,
}

pub struct Forecast {
    pub city: String,
    pub days: Vec<DayForecast>,
}

impl fmt::Display for Forecast {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Forecast for {}", self.city)?;
        writeln!(f, "{:<12} | {:<8} | {:<8} | {:?}", "Date", "High", "Low", "Condition")?;
        writeln!(f, "{}", "-".repeat(40))?;

        for day in &self.days {
            writeln!(f, "{:<12} | {:<8} | {:<8} | {}", day.date, day.high, day.low, day.condition)?;
        }

        Ok(())
    }
}
