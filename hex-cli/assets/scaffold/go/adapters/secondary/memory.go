// Package secondary holds adapters driven by the application.
//
// Rule 4: an adapter imports ports only, never another adapter. Rule 5 is the
// other half: nothing imports this except the composition root.
//
// It sits outside internal/ on purpose. hex's layer classifier treats
// everything under internal/ as private business logic, so an adapter buried
// there would be graded as a use case.
package secondary

import (
	"{{name}}/internal/ports"
)

// InMemoryCounterStore keeps the count in memory.
//
// Swap it for a file or a database by writing another CounterStore and
// changing one line in the composition root. No use case changes.
type InMemoryCounterStore struct {
	count ports.Count
}

// NewInMemoryCounterStore returns a store starting at zero.
func NewInMemoryCounterStore() *InMemoryCounterStore {
	return &InMemoryCounterStore{}
}

func (s *InMemoryCounterStore) Load() ports.Count  { return s.count }
func (s *InMemoryCounterStore) Save(c ports.Count) { s.count = c }
