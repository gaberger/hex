// Use case layer — imports domain + ports only
import type { Weather, FavoriteCity } from "../domain/index.js";
import { createFavoriteCity } from "../domain/index.js";
import type { IWeatherProvider, IFavoritesStore } from "../ports/index.js";

export class WeatherService {
  constructor(
    private readonly weatherProvider: IWeatherProvider,
    private readonly favoritesStore: IFavoritesStore,
  ) {}

  async getWeather(city: string): Promise<Weather> {
    return this.weatherProvider.getWeatherByCity(city);
  }

  async getFavorites(): Promise<FavoriteCity[]> {
    return this.favoritesStore.list();
  }

  async addFavorite(city: string, country: string): Promise<void> {
    const favorite = createFavoriteCity(city, country);
    await this.favoritesStore.add(favorite);
  }

  async removeFavorite(id: string): Promise<void> {
    await this.favoritesStore.remove(id);
  }

  async getFavoritesWeather(): Promise<Weather[]> {
    const favorites = await this.favoritesStore.list();
    const results = await Promise.allSettled(
      favorites.map((f) => this.weatherProvider.getWeatherByCity(f.city)),
    );
    return results
      .filter((r): r is PromiseFulfilledResult<Weather> => r.status === "fulfilled")
      .map((r) => r.value);
  }
}
