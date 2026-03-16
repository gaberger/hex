import { describe, it, expect, mock } from "bun:test";
import { WeatherService } from "../../src/core/usecases/weather-service.js";
import type { IWeatherProvider, IFavoritesStore } from "../../src/core/ports/index.js";
import type { Weather, FavoriteCity } from "../../src/core/domain/index.js";

function mockWeather(city = "London"): Weather {
  return {
    location: { city, country: "GB", lat: 51.5, lon: -0.1 },
    temperature: 15,
    feelsLike: 13,
    humidity: 72,
    windSpeed: 5.1,
    description: "overcast clouds",
    icon: "04d",
    timestamp: 1700000000,
  };
}

function mockProvider(weather?: Weather): IWeatherProvider {
  return {
    getWeatherByCity: mock(async (city: string) => weather ?? mockWeather(city)),
  };
}

function mockStore(initial: FavoriteCity[] = []): IFavoritesStore & { data: FavoriteCity[] } {
  const data = [...initial];
  return {
    data,
    list: mock(async () => [...data]),
    add: mock(async (f: FavoriteCity) => { data.push(f); }),
    remove: mock(async (id: string) => {
      const idx = data.findIndex((f) => f.id === id);
      if (idx >= 0) data.splice(idx, 1);
    }),
  };
}

describe("WeatherService", () => {
  it("returns weather for a city", async () => {
    const service = new WeatherService(mockProvider(), mockStore());
    const result = await service.getWeather("London");
    expect(result.location.city).toBe("London");
    expect(result.temperature).toBe(15);
  });

  it("adds and lists favorites", async () => {
    const store = mockStore();
    const service = new WeatherService(mockProvider(), store);

    await service.addFavorite("Paris", "FR");
    const favorites = await service.getFavorites();

    expect(favorites).toHaveLength(1);
    expect(favorites[0].city).toBe("Paris");
    expect(favorites[0].country).toBe("FR");
  });

  it("removes a favorite", async () => {
    const store = mockStore([{ id: "london-gb", city: "London", country: "GB", addedAt: 1 }]);
    const service = new WeatherService(mockProvider(), store);

    await service.removeFavorite("london-gb");
    const favorites = await service.getFavorites();

    expect(favorites).toHaveLength(0);
  });

  it("gets weather for all favorites, skipping failures", async () => {
    const store = mockStore([
      { id: "london-gb", city: "London", country: "GB", addedAt: 1 },
      { id: "badcity-xx", city: "BadCity", country: "XX", addedAt: 2 },
    ]);
    const provider: IWeatherProvider = {
      getWeatherByCity: mock(async (city: string) => {
        if (city === "BadCity") throw new Error("Not found");
        return mockWeather(city);
      }),
    };
    const service = new WeatherService(provider, store);

    const results = await service.getFavoritesWeather();
    expect(results).toHaveLength(1);
    expect(results[0].location.city).toBe("London");
  });
});
