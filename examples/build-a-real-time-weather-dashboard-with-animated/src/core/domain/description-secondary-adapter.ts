import { WeatherData } from '../domain/weather-data.js';
import { WeatherService } from '../ports/weather-service.js';
import axios from 'axios';

export class DescriptionSecondaryAdapter implements WeatherService {
    private readonly apiKey: string;
    private readonly apiUrl: string;

    constructor(apiKey: string, apiUrl: string) {
        this.apiKey = apiKey;
        this.apiUrl = apiUrl;
    }

    async fetchWeather(city: string): Promise<WeatherData> {
        const response = await axios.get(`${this.apiUrl}?q=${city}&appid=${this.apiKey}`);
        const data = response.data;

        return {
            temperature: data.main.temp,
            description: data.weather[0].description,
            city: data.name,
        } as WeatherData;
    }
}