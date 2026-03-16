package main

import (
	"testing"

	"hex-f1/src/adapters/primary"
	"hex-f1/src/adapters/secondary"
	"hex-f1/src/core/usecases"
)

// TestCompositionRoot_Wiring verifies the composition root can wire all
// adapters together without panicking. This is a build-level smoke test
// that catches interface mismatches at test time rather than at startup.
func TestCompositionRoot_Wiring(t *testing.T) {
	jolpica := secondary.NewJolpicaAdapter()
	cache := secondary.NewCacheAdapter()
	f1Service := usecases.NewF1Service(jolpica, cache)
	adapter := primary.NewHTTPAdapter(f1Service)

	if adapter == nil {
		t.Fatal("expected non-nil HTTPAdapter")
	}

	handler := adapter.Handler()
	if handler == nil {
		t.Fatal("expected non-nil http.Handler")
	}
}

// TestCompositionRoot_NilCache verifies the app can start without a cache.
func TestCompositionRoot_NilCache(t *testing.T) {
	jolpica := secondary.NewJolpicaAdapter()
	f1Service := usecases.NewF1Service(jolpica, nil)
	adapter := primary.NewHTTPAdapter(f1Service)

	if adapter == nil {
		t.Fatal("expected non-nil HTTPAdapter without cache")
	}
}
