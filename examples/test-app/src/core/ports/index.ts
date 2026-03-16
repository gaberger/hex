// Port interfaces — imports only from domain
import type { Weather, FavoriteCity } from "../domain/index.js";

export interface IWeatherProvider {
  getWeatherByCity(city: string): Promise<Weather>;
}

export interface IFavoritesStore {
  list(): Promise<FavoriteCity[]>;
  add(favorite: FavoriteCity): Promise<void>;
  remove(id: string): Promise<void>;
}

export interface IHttpServer {
  start(port: number): Promise<void>;
  stop(): Promise<void>;
}
