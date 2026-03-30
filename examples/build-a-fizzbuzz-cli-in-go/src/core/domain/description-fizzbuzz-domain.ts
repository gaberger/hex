import { FizzBuzzResult } from './fizzbuzz-result.js';

export class FizzBuzz {
    static compute(start: number, end: number): FizzBuzzResult[] {
        const results: FizzBuzzResult[] = [];
        
        for (let i = start; i <= end; i++) {
            if (i % 15 === 0) {
                results.push(new FizzBuzzResult(i, 'FizzBuzz'));
            } else if (i % 3 === 0) {
                results.push(new FizzBuzzResult(i, 'Fizz'));
            } else if (i % 5 === 0) {
                results.push(new FizzBuzzResult(i, 'Buzz'));
            } else {
                results.push(new FizzBuzzResult(i, i.toString()));
            }
        }
        
        return results;
    }
}