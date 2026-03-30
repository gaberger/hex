export interface DescriptionService {
    getDescription(id: string): Promise<string>;
    createDescription(content: string): Promise<string>;
    updateDescription(id: string, content: string): Promise<void>;
    deleteDescription(id: string): Promise<void>;
}