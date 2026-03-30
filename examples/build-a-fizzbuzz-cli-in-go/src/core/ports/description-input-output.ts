export interface DescriptionInput {
    title: string;
    content: string;
}

export interface DescriptionOutput {
    id: string;
    title: string;
    content: string;
    createdAt: Date;
    updatedAt: Date;
}