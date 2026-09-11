// Package usecases holds the application's verbs.
//
// Rule 3: usecases imports domain and ports only. It takes the port as a
// parameter and never chooses which adapter fills it; that choice belongs to
// the composition root alone.
package usecases

import (
	"{{name}}/internal/domain"
	"{{name}}/internal/ports"
)

// Increment advances the count by one and returns the new value.
func Increment(store ports.CounterStore) domain.Count {
	next := store.Load().Next()
	store.Save(next)
	return next
}
