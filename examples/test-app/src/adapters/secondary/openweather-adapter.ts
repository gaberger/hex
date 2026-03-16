// Secondary adapter — implements IWeatherProvider via OpenWeather API
// Imports only from ports
import type { IWeatherProvider } from "../../core/ports/index.js";
import type { Weather } from "../../core/domain/index.js";

export class OpenWeatherAdapter implements IWeatherProvider {
  private readonly baseUrl = "https://api.openweathermap.org/data/2.5";

  constructor(private readonly apiKey: string) {}

  async getWeatherByCity(city: string): Promise<Weather> {
    const url = `${this.baseUrl}/weather?q=${encodeURIComponent(city)}&appid=${this.apiKey}&units=metric`;
    const response = await fetch(url);

    if (!response.ok) {
      const body = await response.text();
      throw new Error(`OpenWeather API error (${response.status}): ${body}`);
    }

    const data = await response.json();
    return {
      location: {
        city: data.name,
        country: data.sys.country,
        lat: data.coord.lat,
        lon: data.coord.lon,
      },
      temperature: data.main.temp,
      feelsLike: data.main.feels_like,
      humidity: data.main.humidity,
      windSpeed: data.wind.speed,
      description: data.weather[0].description,
      icon: data.weather[0].icon,
      timestamp: data.dt,
    };
  }
}
