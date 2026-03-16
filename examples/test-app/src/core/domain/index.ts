// Domain entities and value objects — zero external dependencies

export interface Location {
  city: string;
  country: string;
  lat: number;
  lon: number;
}

export interface Weather {
  location: Location;
  temperature: number; // Celsius
  feelsLike: number;
  humidity: number; // percentage
  windSpeed: number; // m/s
  description: string;
  icon: string; // OpenWeather icon code
  timestamp: number; // Unix timestamp
}

export interface FavoriteCity {
  id: string;
  city: string;
  country: string;
  addedAt: number;
}

export function createFavoriteCity(city: string, country: string): FavoriteCity {
  return {
    id: `${city.toLowerCase()}-${country.toLowerCase()}`,
    city,
    country,
    addedAt: Date.now(),
  };
}
