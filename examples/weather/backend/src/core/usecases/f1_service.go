package usecases

import (
	"context"
	"encoding/json"
	"fmt"
	"time"

	"hex-f1/src/core/domain"
	"hex-f1/src/core/ports"
)

// F1Service orchestrates F1 data retrieval through ports.
// It uses constructor injection for the data port and an optional cache port.
type F1Service struct {
	dataPort ports.IF1DataPort
	cache    ports.ICachePort
}

// NewF1Service creates a new F1Service. The cache parameter may be nil.
func NewF1Service(dataPort ports.IF1DataPort, cache ports.ICachePort) *F1Service {
	return &F1Service{
		dataPort: dataPort,
		cache:    cache,
	}
}

// GetCurrentSchedule returns the race schedule for the current season.
func (s *F1Service) GetCurrentSchedule(ctx context.Context) (*domain.SeasonSchedule, error) {
	season := domain.Season(time.Now().Year())
	cacheKey := fmt.Sprintf("schedule:%d", season)

	if cached, ok := s.getCache(ctx, cacheKey); ok {
		var schedule domain.SeasonSchedule
		if err := json.Unmarshal(cached, &schedule); err == nil {
			return &schedule, nil
		}
	}

	schedule, err := s.dataPort.GetSeasonSchedule(ctx, season)
	if err != nil {
		return nil, fmt.Errorf("get current schedule: %w", err)
	}

	s.setCache(ctx, cacheKey, schedule)
	return schedule, nil
}

// GetRaceResult returns the result for a specific race identified by season and round.
func (s *F1Service) GetRaceResult(ctx context.Context, season domain.Season, round domain.RoundNumber) (*domain.RaceResult, error) {
	cacheKey := fmt.Sprintf("race:%d:%d", season, round)

	if cached, ok := s.getCache(ctx, cacheKey); ok {
		var result domain.RaceResult
		if err := json.Unmarshal(cached, &result); err == nil {
			return &result, nil
		}
	}

	result, err := s.dataPort.GetRaceResult(ctx, season, round)
	if err != nil {
		return nil, fmt.Errorf("get race result (season=%d, round=%d): %w", season, round, err)
	}

	s.setCache(ctx, cacheKey, result)
	return result, nil
}

// GetLatestResult returns the most recent completed race result.
func (s *F1Service) GetLatestResult(ctx context.Context) (*domain.RaceResult, error) {
	cacheKey := "race:latest"

	if cached, ok := s.getCache(ctx, cacheKey); ok {
		var result domain.RaceResult
		if err := json.Unmarshal(cached, &result); err == nil {
			return &result, nil
		}
	}

	result, err := s.dataPort.GetLatestRaceResult(ctx)
	if err != nil {
		return nil, fmt.Errorf("get latest race result: %w", err)
	}

	s.setCache(ctx, cacheKey, result)
	return result, nil
}

// GetDriverStandings returns the driver championship standings for a given season.
func (s *F1Service) GetDriverStandings(ctx context.Context, season domain.Season) ([]domain.DriverStanding, error) {
	cacheKey := fmt.Sprintf("standings:drivers:%d", season)

	if cached, ok := s.getCache(ctx, cacheKey); ok {
		var standings []domain.DriverStanding
		if err := json.Unmarshal(cached, &standings); err == nil {
			return standings, nil
		}
	}

	standings, err := s.dataPort.GetDriverStandings(ctx, season)
	if err != nil {
		return nil, fmt.Errorf("get driver standings (season=%d): %w", season, err)
	}

	s.setCache(ctx, cacheKey, standings)
	return standings, nil
}

// GetConstructorStandings returns the constructor championship standings for a given season.
func (s *F1Service) GetConstructorStandings(ctx context.Context, season domain.Season) ([]domain.ConstructorStanding, error) {
	cacheKey := fmt.Sprintf("standings:constructors:%d", season)

	if cached, ok := s.getCache(ctx, cacheKey); ok {
		var standings []domain.ConstructorStanding
		if err := json.Unmarshal(cached, &standings); err == nil {
			return standings, nil
		}
	}

	standings, err := s.dataPort.GetConstructorStandings(ctx, season)
	if err != nil {
		return nil, fmt.Errorf("get constructor standings (season=%d): %w", season, err)
	}

	s.setCache(ctx, cacheKey, standings)
	return standings, nil
}

// GetFullStandings returns both driver and constructor standings for a given season.
func (s *F1Service) GetFullStandings(ctx context.Context, season domain.Season) (*domain.StandingsResponse, error) {
	cacheKey := fmt.Sprintf("standings:full:%d", season)

	if cached, ok := s.getCache(ctx, cacheKey); ok {
		var resp domain.StandingsResponse
		if err := json.Unmarshal(cached, &resp); err == nil {
			return &resp, nil
		}
	}

	drivers, err := s.dataPort.GetDriverStandings(ctx, season)
	if err != nil {
		return nil, fmt.Errorf("get full standings — drivers (season=%d): %w", season, err)
	}

	constructors, err := s.dataPort.GetConstructorStandings(ctx, season)
	if err != nil {
		return nil, fmt.Errorf("get full standings — constructors (season=%d): %w", season, err)
	}

	resp := &domain.StandingsResponse{
		Season:               season,
		DriverStandings:      drivers,
		ConstructorStandings: constructors,
	}

	s.setCache(ctx, cacheKey, resp)
	return resp, nil
}

// getCache is a nil-safe helper that reads from the cache port.
func (s *F1Service) getCache(ctx context.Context, key string) ([]byte, bool) {
	if s.cache == nil {
		return nil, false
	}
	return s.cache.Get(ctx, key)
}

// setCache is a nil-safe helper that writes to the cache port.
func (s *F1Service) setCache(ctx context.Context, key string, value any) {
	if s.cache == nil {
		return
	}
	data, err := json.Marshal(value)
	if err != nil {
		return
	}
	s.cache.Set(ctx, key, data)
}
