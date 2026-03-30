export interface WeatherDataPort {
    fetchWeatherData(location: string): Promise<WeatherData>;
}

export interface GeolocationPort {
    fetchGeolocation(ipAddress: string): Promise<Geolocation>;
}

export interface WeatherData {
    temperature: number;
    description: string;
    humidity: number;
    windSpeed: number;
}

export interface Geolocation {
    city: string;
    region: string;
    country: string;
    latitude: number;
    longitude: number;
}