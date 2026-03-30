import { ValueObject } from './value-object';

export class FizzBuzzResult extends ValueObject {
    constructor(private value: string) {
        super();
    }

    public getValue(): string {
        return this.value;
    }

    public static fromNumber(num: number): FizzBuzzResult {
        let result = '';
        if (num % 3 === 0) {
            result += 'Fizz';
        }
        if (num % 5 === 0) {
            result += 'Buzz';
        }
        return new FizzBuzzResult(result || num.toString());
    }
}