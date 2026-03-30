import { FizzBuzzInput, FizzBuzzOutput } from './value-objects.js';
import { FizzBuzzService } from '../ports/fizzbuzz-service.js';

export class FizzBuzzUseCase {
    constructor(private fizzBuzzService: FizzBuzzService) {}

    execute(input: FizzBuzzInput): FizzBuzzOutput {
        const result = this.fizzBuzzService.calculate(input);
        return result;
    }

    handleUserInput(input: string): FizzBuzzOutput {
        const parsedInput = this.parseInput(input);
        return this.execute(parsedInput);
    }

    private parseInput(input: string): FizzBuzzInput {
        // Implementation for parsing user input to FizzBuzzInput
        const number = parseInt(input, 10);
        if (isNaN(number)) {
            throw new Error('Invalid input, please enter a number.');
        }
        return { number };
    }
}