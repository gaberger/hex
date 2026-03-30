package main

import(
	"flag"
	"fmt"
)

func main() {
	n := flag.Int("n", 100, "the number to fizzbuzz up to")
	flag.Parse()
	for i := 1; i <= *n; i++ {
		if i%15 == 0 {
			fmt.Println("FizzBuzz")
		} else if i%5 == 0 {
			fmt.Println("Buzz")
		} else if i%3 == 0 {
			fmt.Println("Fizz")
		} else {
			fmt.Println(i)
		}
	}
}