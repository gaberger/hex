// Package domain holds value objects and entities. Pure data, no I/O.
//
// Rule 1: domain imports only domain. Nothing here may reach for a port, a
// use case, or an adapter. That is what makes it testable with no wiring.
package domain

// Count is how many times something has happened. Never negative, by
// construction — the zero value is a valid, empty Count.
type Count struct {
	value uint64
}

// Zero is the starting count.
func Zero() Count { return Count{} }

// Next returns the following count. It saturates rather than wrapping: a
// counter that silently restarts at zero is worse than one that stops.
func (c Count) Next() Count {
	if c.value == ^uint64(0) {
		return c
	}
	return Count{value: c.value + 1}
}

// Value is the count as a number.
func (c Count) Value() uint64 { return c.value }
