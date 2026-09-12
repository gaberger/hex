// Package {{name_snake}} is the composition root: the only place allowed to
// name a concrete adapter. Everything else depends on the port.
//
//	  domain  ←  ports  ←  usecases
//	               ↑
//	          adapters (secondary)
//	               ↑
//	   composition-root.go — wires them, once
//
// Check it with `hex analyze .`.
package {{name_snake}}

import (
	"{{name}}/adapters/secondary"
	"{{name}}/internal/domain"
	"{{name}}/internal/ports"
	"{{name}}/internal/usecases"
)

// Counter builds the application with its real adapters.
//
// The one line below is the whole composition decision. Point it at a
// file-backed store and nothing else moves.
func Counter() ports.CounterStore {
	return secondary.NewInMemoryCounterStore()
}

// IncrementOnce runs the use case against a freshly composed application.
func IncrementOnce() domain.Count {
	return usecases.Increment(Counter())
}
