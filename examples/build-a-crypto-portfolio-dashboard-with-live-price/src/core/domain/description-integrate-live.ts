import { PriceUpdate } from "./price-update.js";
import { LiveDataSource } from "../ports/live-data-source.js";

export class LiveDataIntegration {
    private dataSource: LiveDataSource;

    constructor(dataSource: LiveDataSource) {
        this.dataSource = dataSource;
    }

    async getRealTimePriceUpdates(): Promise<void> {
        this.dataSource.onPriceUpdate((update: PriceUpdate) => {
            this.handlePriceUpdate(update);
        });
    }

    private handlePriceUpdate(update: PriceUpdate): void {
        // Handle the price update (e.g., update the dashboard)
        console.log(`Price updated: ${update.price}`);
    }
}