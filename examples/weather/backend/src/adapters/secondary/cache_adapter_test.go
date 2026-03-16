package secondary_test

import (
	"context"
	"testing"

	"hex-f1/src/adapters/secondary"
)

func TestCacheAdapter_GetMiss(t *testing.T) {
	cache := secondary.NewCacheAdapter()
	val, ok := cache.Get(context.Background(), "nonexistent")

	if ok {
		t.Error("expected cache miss, got hit")
	}
	if val != nil {
		t.Errorf("expected nil value, got %v", val)
	}
}

func TestCacheAdapter_SetThenGet(t *testing.T) {
	cache := secondary.NewCacheAdapter()
	ctx := context.Background()

	data := []byte(`{"season":2024}`)
	cache.Set(ctx, "schedule:2024", data)

	val, ok := cache.Get(ctx, "schedule:2024")
	if !ok {
		t.Fatal("expected cache hit, got miss")
	}
	if string(val) != string(data) {
		t.Errorf("got %q, want %q", string(val), string(data))
	}
}

func TestCacheAdapter_OverwriteExistingKey(t *testing.T) {
	cache := secondary.NewCacheAdapter()
	ctx := context.Background()

	cache.Set(ctx, "key", []byte("first"))
	cache.Set(ctx, "key", []byte("second"))

	val, ok := cache.Get(ctx, "key")
	if !ok {
		t.Fatal("expected cache hit")
	}
	if string(val) != "second" {
		t.Errorf("got %q, want %q", string(val), "second")
	}
}

func TestCacheAdapter_IndependentKeys(t *testing.T) {
	cache := secondary.NewCacheAdapter()
	ctx := context.Background()

	cache.Set(ctx, "a", []byte("alpha"))
	cache.Set(ctx, "b", []byte("beta"))

	valA, okA := cache.Get(ctx, "a")
	valB, okB := cache.Get(ctx, "b")

	if !okA || string(valA) != "alpha" {
		t.Errorf("key 'a': got %q (ok=%v), want 'alpha'", string(valA), okA)
	}
	if !okB || string(valB) != "beta" {
		t.Errorf("key 'b': got %q (ok=%v), want 'beta'", string(valB), okB)
	}
}
