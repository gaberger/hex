// This file is for defining the ports that the primary adapters will implement.
// It acts as an interface for communication between the domain logic and the UI components.

export interface DataVisualizer {
    visualize(data: any): void; // Method to visualize data
}

export interface LiveDataProvider {
    fetchData(): Promise<any>; // Method to fetch live data
}