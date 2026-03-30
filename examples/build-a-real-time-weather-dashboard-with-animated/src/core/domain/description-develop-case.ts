import { WeatherData } from './weather-data.js';
import { GeoLocation } from './geo-location.js';
import { WeatherRepository } from '../ports/weather-repository.js';

export class FetchWeatherUseCase {
    constructor(private weatherRepository: WeatherRepository) {}

    async execute(location: GeoLocation): Promise<WeatherData> {
        const weatherData = await this.weatherRepository.fetchWeather(location);
        return this.presentWeatherData(weatherData);
    }

    private presentWeatherData(weatherData: WeatherData): WeatherData {
        // Process and format weather data for presentation
        return weatherData; // You can modify this to return a more appropriate presentation format
    }
}