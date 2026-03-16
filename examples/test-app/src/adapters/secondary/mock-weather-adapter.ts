// Secondary adapter — mock IWeatherProvider for development without API key
// Imports only from ports
import type { IWeatherProvider } from "../../core/ports/index.js";
import type { Weather } from "../../core/domain/index.js";

const MOCK_CITIES: Record<string, Weather> = {
  "new york": {
    location: { city: "New York", country: "US", lat: 40.71, lon: -74.01 },
    temperature: 22, feelsLike: 20, humidity: 55, windSpeed: 4.1,
    description: "partly cloudy", icon: "02d", timestamp: Date.now(),
  },
  london: {
    location: { city: "London", country: "GB", lat: 51.51, lon: -0.13 },
    temperature: 15, feelsLike: 13, humidity: 72, windSpeed: 3.6,
    description: "light rain", icon: "10d", timestamp: Date.now(),
  },
  tokyo: {
    location: { city: "Tokyo", country: "JP", lat: 35.68, lon: 139.69 },
    temperature: 28, feelsLike: 31, humidity: 80, windSpeed: 2.1,
    description: "clear sky", icon: "01d", timestamp: Date.now(),
  },
  paris: {
    location: { city: "Paris", country: "FR", lat: 48.86, lon: 2.35 },
    temperature: 18, feelsLike: 17, humidity: 60, windSpeed: 5.2,
    description: "scattered clouds", icon: "03d", timestamp: Date.now(),
  },
  sydney: {
    location: { city: "Sydney", country: "AU", lat: -33.87, lon: 151.21 },
    temperature: 20, feelsLike: 19, humidity: 65, windSpeed: 6.7,
    description: "overcast clouds", icon: "04d", timestamp: Date.now(),
  },
};

export class MockWeatherAdapter implements IWeatherProvider {
  async getWeatherByCity(city: string): Promise<Weather> {
    const key = city.toLowerCase().trim();
    const match = MOCK_CITIES[key];

    if (match) {
      return { ...match, timestamp: Math.floor(Date.now() / 1000) };
    }

    // Generate plausible data for any unknown city
    return {
      location: { city, country: "XX", lat: 0, lon: 0 },
      temperature: 15 + Math.round(Math.random() * 20),
      feelsLike: 13 + Math.round(Math.random() * 20),
      humidity: 40 + Math.round(Math.random() * 50),
      windSpeed: Math.round(Math.random() * 10 * 10) / 10,
      description: "few clouds",
      icon: "02d",
      timestamp: Math.floor(Date.now() / 1000),
    };
  }
}
