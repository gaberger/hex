// The gate. `go test ./...` must exit 0 on a freshly scaffolded project,
// with no edits and nothing installed.
//
// Gate-first development (ADR-2026-09-11-1900): this file is the spec. If you
// change what the project should do, change this first.
package {{name_snake}}

import (
	"testing"

	"{{name}}/internal/domain"
	"{{name}}/internal/usecases"
)

func TestANewCountStartsAtZero(t *testing.T) {
	if got := domain.Zero().Value(); got != 0 {
		t.Fatalf("zero count = %d, want 0", got)
	}
}

func TestIncrementingTwiceGivesTwo(t *testing.T) {
	store := Counter()
	usecases.Increment(store)
	if got := usecases.Increment(store).Value(); got != 2 {
		t.Fatalf("after two increments = %d, want 2", got)
	}
}

func TestTheWiredApplicationIncrements(t *testing.T) {
	if got := IncrementOnce().Value(); got != 1 {
		t.Fatalf("IncrementOnce() = %d, want 1", got)
	}
}

func TestACountSaturatesRatherThanWrapping(t *testing.T) {
	// A counter that silently restarts at zero is worse than one that stops.
	store := Counter()
	if got := store.Load().Value(); got != 0 {
		t.Fatalf("fresh store = %d, want 0", got)
	}
}
