package ports

import (
	"context"

	"hex-f1/src/core/domain"
)

// IF1DataPort is the secondary port for fetching F1 data from an external source.
// Adapters: Jolpica API, OpenF1, mock, etc.
type IF1DataPort interface {
	// GetSeasonSchedule returns all races for a given season.
	GetSeasonSchedule(ctx context.Context, season domain.Season) (*domain.SeasonSchedule, error)

	// GetRaceResult returns full results for a specific race.
	GetRaceResult(ctx context.Context, season domain.Season, round domain.RoundNumber) (*domain.RaceResult, error)

	// GetLatestRaceResult returns the most recent completed race result.
	GetLatestRaceResult(ctx context.Context) (*domain.RaceResult, error)

	// GetDriverStandings returns current driver championship standings.
	GetDriverStandings(ctx context.Context, season domain.Season) ([]domain.DriverStanding, error)

	// GetConstructorStandings returns current constructor championship standings.
	GetConstructorStandings(ctx context.Context, season domain.Season) ([]domain.ConstructorStanding, error)
}

// IF1QueryPort is the primary port exposing F1 queries to driving adapters.
// The HTTP adapter depends on this interface, not a concrete usecase.
type IF1QueryPort interface {
	GetCurrentSchedule(ctx context.Context) (*domain.SeasonSchedule, error)
	GetRaceResult(ctx context.Context, season domain.Season, round domain.RoundNumber) (*domain.RaceResult, error)
	GetLatestResult(ctx context.Context) (*domain.RaceResult, error)
	GetDriverStandings(ctx context.Context, season domain.Season) ([]domain.DriverStanding, error)
	GetConstructorStandings(ctx context.Context, season domain.Season) ([]domain.ConstructorStanding, error)
}

// IHTTPServerPort is the primary port for serving the web UI.
// The composition root wires this to the HTTP adapter.
type IHTTPServerPort interface {
	// Start begins listening on the given address (e.g. ":8080").
	Start(addr string) error

	// Stop gracefully shuts down the server.
	Stop(ctx context.Context) error
}

// ICachePort is an optional secondary port for caching API responses.
type ICachePort interface {
	Get(ctx context.Context, key string) ([]byte, bool)
	Set(ctx context.Context, key string, value []byte)
}
