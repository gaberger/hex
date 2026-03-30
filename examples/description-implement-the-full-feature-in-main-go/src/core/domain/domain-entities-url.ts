import { ValueObject } from './value-objects'; // Assume this is a base value object structure

export class URL extends ValueObject<string> {
    constructor(value: string) {
        super(value);
        this.validate(value);
    }

    private validate(value: string): void {
        const urlPattern = new RegExp(/^(https?:\/\/[^\s/$.?#].[^\s]*)$/i);
        if (!urlPattern.test(value)) {
            throw new Error(`Invalid URL: ${value}`);
        }
    }
}

export class ShortLink extends ValueObject<string> {
    constructor(value: string) {
        super(value);
        this.validate(value);
    }

    private validate(value: string): void {
        const shortLinkPattern = new RegExp(/^[a-zA-Z0-9]{6,}$/);
        if (!shortLinkPattern.test(value)) {
            throw new Error(`Invalid ShortLink: ${value}`);
        }
    }
}

export interface UrlMapping {
    id: string;
    originalUrl: URL;
    shortLink: ShortLink;
}