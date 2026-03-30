// This file serves as a placeholder for secondary adapters related to data storage or retrieval.
// Actual implementation would depend on specific data storage or retrieval requirements in the project. 

export interface DescriptionRepository {
    // Define methods that the secondary adapter should implement
    findById(id: string): Promise<Description | null>;
    save(description: Description): Promise<void>;
}

export interface Description {
    id: string;
    content: string;
}