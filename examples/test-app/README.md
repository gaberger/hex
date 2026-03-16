# Weather App

A hexagonal architecture weather app using the OpenWeather API, built with Bun + SQLite.

## Quick Start

```bash
bun install
cp .env.example .env          # Add your OpenWeather API key
OPENWEATHER_API_KEY=your_key bun run dev
```

Open http://localhost:3000 in your browser.

Get a free API key at https://openweathermap.org/api

## Commands

| Command | Description |
|---------|-------------|
| `bun run dev` | Start dev server with watch |
| `bun run start` | Start without watch |
| `bun test` | Run tests |
| `bun run build` | Build for production |
| `bun run check` | Type-check without emitting |

## Architecture

```
src/
  core/
    domain/index.ts              Weather, Location, FavoriteCity
    ports/index.ts               IWeatherProvider, IFavoritesStore, IHttpServer
    usecases/weather-service.ts  Business logic orchestration
  adapters/
    primary/http-adapter.ts      Bun HTTP server + web UI
    secondary/
      openweather-adapter.ts     OpenWeather API client
      sqlite-storage.ts          SQLite favorites persistence
  composition-root.ts            Wires adapters to ports (single DI point)
  index.ts                       Entry point
```

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/weather?city=London` | Get current weather |
| GET | `/api/favorites` | List favorite cities |
| POST | `/api/favorites` | Add favorite `{city, country}` |
| DELETE | `/api/favorites?id=london-gb` | Remove favorite |
| GET | `/api/favorites/weather` | Weather for all favorites |
