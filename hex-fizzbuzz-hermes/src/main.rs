{
  "feature": "examples-hex-fizzbuzz-hermes",
  "description": "Hexagonal fizzbuzz example built by the hex autonomous SOP loop as the Phase 0+ proof for ADR-2605141135.",
  "behavioral_specs": [
    {
      "id": "BS-1",
      "name": "fizzbuzz domain function — single number",
      "given": "an integer n",
      "when": "fizzbuzz(n) is called",
      "then": "returns 'Fizz' when n % 3 == 0 and n % 5 != 0; 'Buzz' when n % 5 == 0 and n % 3 != 0; 'FizzBuzz' when n % 15 == 0; the decimal string of n otherwise"
    },
    {
      "id": "BS-2",
      "name": "play usecase — iterates inclusive range",
      "given": "a Writer and integers start <= end",
      "when": "play(writer, start, end) is called",
      "then": "writer.write is invoked exactly (end - start + 1) times in order with fizzbuzz(start), fizzbuzz(start+1), ..., fizzbuzz(end)"
    },
    {
      "id": "BS-3",
      "name": "CLI adapter — default range",
      "given": "the fizzbuzz binary invoked with no arguments",
      "when": "main runs",
      "then": "stdout receives 15 lines: 1, 2, Fizz, 4, Buzz, Fizz, 7, 8, Fizz, Buzz, 11, Fizz, 13, 14, FizzBuzz"
    },
    {
      "id": "BS-4",
      "name": "CLI adapter — invalid range rejected",
      "given": "the fizzbuzz binary invoked with end < start",
      "when": "main runs",
      "then": "process exits with code 2 and stderr contains 'usage: fizzbuzz'"
    }
  ],
  "verification": "cargo test from examples/hex-fizzbuzz-hermes/ must pass; cargo run -- 1 5 must emit exactly: 1\\n2\\nFizz\\n4\\nBuzz"
}