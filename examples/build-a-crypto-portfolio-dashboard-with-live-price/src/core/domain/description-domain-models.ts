export class CryptoAsset {
    constructor(
        public readonly id: string, 
        public readonly name: string, 
        public readonly symbol: string, 
        public readonly value: ValueObject
    ) {}
}

export class ValueObject {
    constructor(
        public readonly amount: number, 
        public readonly currency: string
    ) {}

    isEqual(other: ValueObject): boolean {
        return this.amount === other.amount && this.currency === other.currency;
    }
}