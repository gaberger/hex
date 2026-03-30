import { FizzBuzzType } from './fizzbuzz-type.js';

export class FizzBuzz {
    static compute(n: number): FizzBuzzType {
        if (n % 15 === 0) {
            return FizzBuzzType.FIZZBUZZ;
        } else if (n % 3 === 0) {
            return FizzBuzzType.FIZZ;
        } else if (n % 5 === 0) {
            return FizzBuzzType.BUZZ;
        } else {
            return FizzBuzzType.NUMBER;
        }
    }
}