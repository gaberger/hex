// Composition root — the ONLY file that imports from adapters
import { OpenWeatherAdapter } from "./adapters/secondary/openweather-adapter.js";
import { MockWeatherAdapter } from "./adapters/secondary/mock-weather-adapter.js";
import { SqliteFavoritesStore } from "./adapters/secondary/sqlite-storage.js";
import { HttpAdapter } from "./adapters/primary/http-adapter.js";
import { WeatherService } from "./core/usecases/weather-service.js";

export interface AppConfig {
  openWeatherApiKey?: string;
  dbPath: string;
  port: number;
}

export function createApp(config: AppConfig) {
  const weatherProvider = config.openWeatherApiKey
    ? new OpenWeatherAdapter(config.openWeatherApiKey)
    : new MockWeatherAdapter();
  const favoritesStore = new SqliteFavoritesStore(config.dbPath);
  const weatherService = new WeatherService(weatherProvider, favoritesStore);
  const httpServer = new HttpAdapter(weatherService);

  return {
    start: () => httpServer.start(config.port),
    stop: () => httpServer.stop(),
  };
}
