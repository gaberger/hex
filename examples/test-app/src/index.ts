import { createApp } from "./composition-root.js";

const apiKey = process.env.OPENWEATHER_API_KEY;
if (!apiKey) {
  console.warn("OPENWEATHER_API_KEY not set — using mock weather data.");
  console.warn("Get a free key at https://openweathermap.org/api");
}

const app = createApp({
  openWeatherApiKey: apiKey,
  dbPath: "./weather.db",
  port: Number(process.env.PORT) || 3000,
});

app.start();
