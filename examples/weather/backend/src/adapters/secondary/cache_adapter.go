package secondary

import (
	"context"
	"sync"

	"hex-f1/src/core/ports"
)

// CacheAdapter implements ports.ICachePort using an in-memory sync.Map.
// No TTL — entries persist for the lifetime of the process.
type CacheAdapter struct {
	store sync.Map
}

// Compile-time interface check.
var _ ports.ICachePort = (*CacheAdapter)(nil)

// NewCacheAdapter creates a new in-memory cache.
func NewCacheAdapter() *CacheAdapter {
	return &CacheAdapter{}
}

// Get retrieves a cached value by key. Returns (nil, false) on cache miss.
func (c *CacheAdapter) Get(_ context.Context, key string) ([]byte, bool) {
	val, ok := c.store.Load(key)
	if !ok {
		return nil, false
	}
	data, ok := val.([]byte)
	return data, ok
}

// Set stores a value in the cache, overwriting any existing entry for key.
func (c *CacheAdapter) Set(_ context.Context, key string, value []byte) {
	c.store.Store(key, value)
}
