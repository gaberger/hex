package main

import (
	"context"
	"log"
	"os"
	"os/signal"
	"syscall"
	"time"

	"hex-f1/src/adapters/primary"
	"hex-f1/src/adapters/secondary"
	"hex-f1/src/core/usecases"
)

func main() {
	// --- Secondary Adapters (driven) ---
	jolpica := secondary.NewJolpicaAdapter()
	cache := secondary.NewCacheAdapter()

	// --- Use Cases ---
	f1Service := usecases.NewF1Service(jolpica, cache)

	// --- Primary Adapters (driving) ---
	httpAdapter := primary.NewHTTPAdapter(f1Service)

	// --- Start ---
	addr := ":8080"
	if port := os.Getenv("PORT"); port != "" {
		addr = ":" + port
	}

	// Graceful shutdown on SIGINT/SIGTERM
	stop := make(chan os.Signal, 1)
	signal.Notify(stop, syscall.SIGINT, syscall.SIGTERM)

	go func() {
		if err := httpAdapter.Start(addr); err != nil {
			log.Fatalf("server error: %v", err)
		}
	}()

	<-stop
	log.Println("Shutting down...")

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	if err := httpAdapter.Stop(ctx); err != nil {
		log.Fatalf("shutdown error: %v", err)
	}
	log.Println("Server stopped")
}
