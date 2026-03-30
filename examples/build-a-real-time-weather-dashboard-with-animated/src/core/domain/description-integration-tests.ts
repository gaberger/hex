import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { createWeatherDashboard } from '../weatherDashboard'; // Assuming this is the domain use case to test
import { InMemoryWeatherRepository } from '../../adapters/secondary/inMemoryWeatherRepository'; // In-memory repo for testing

describe('Weather Dashboard Integration Tests', () => {
    let dashboard: ReturnType<typeof createWeatherDashboard>;
    let weatherRepository: InMemoryWeatherRepository;

    beforeAll(() => {
        weatherRepository = new InMemoryWeatherRepository();
        dashboard = createWeatherDashboard(weatherRepository);
    });

    afterAll(() => {
        weatherRepository.clear(); // Assuming there's a clear method to reset state after tests
    });

    it('should display current weather', async () => {
        // Arrange
        await weatherRepository.addWeather({ location: 'New York', temperature: 75, conditions: 'Sunny' });

        // Act
        const currentWeather = await dashboard.getCurrentWeather('New York');

        // Assert
        expect(currentWeather).toEqual({ location: 'New York', temperature: 75, conditions: 'Sunny' });
    });

    it('should handle not found location gracefully', async () => {
        // Act
        const currentWeather = await dashboard.getCurrentWeather('Unknown Town');

        // Assert
        expect(currentWeather).toBeNull();
    });

    it('should display forecast', async () => {
        // Arrange
        await weatherRepository.addForecast({ location: 'Los Angeles', forecast: [{ day: 'Monday', temperature: 80 }] });

        // Act
        const forecast = await dashboard.getForecast('Los Angeles');

        // Assert
        expect(forecast).toEqual([{ day: 'Monday', temperature: 80 }]);
    });
});