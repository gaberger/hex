package usecases_test

import (
	"context"
	"errors"
	"testing"

	"hex-f1/src/core/domain"
	"hex-f1/src/core/usecases"
)

// --- Mock ports (London-school: mock at the boundary) ---

type mockF1DataPort struct {
	scheduleResult    *domain.SeasonSchedule
	raceResult        *domain.RaceResult
	latestResult      *domain.RaceResult
	driverStandings   []domain.DriverStanding
	constructorStandings []domain.ConstructorStanding
	err               error

	// Call tracking
	scheduleCalledWith  domain.Season
	raceCalledSeason    domain.Season
	raceCalledRound     domain.RoundNumber
	latestCalled        bool
	driverCalledWith    domain.Season
	constructorCalledWith domain.Season
}

func (m *mockF1DataPort) GetSeasonSchedule(_ context.Context, season domain.Season) (*domain.SeasonSchedule, error) {
	m.scheduleCalledWith = season
	return m.scheduleResult, m.err
}

func (m *mockF1DataPort) GetRaceResult(_ context.Context, season domain.Season, round domain.RoundNumber) (*domain.RaceResult, error) {
	m.raceCalledSeason = season
	m.raceCalledRound = round
	return m.raceResult, m.err
}

func (m *mockF1DataPort) GetLatestRaceResult(_ context.Context) (*domain.RaceResult, error) {
	m.latestCalled = true
	return m.latestResult, m.err
}

func (m *mockF1DataPort) GetDriverStandings(_ context.Context, season domain.Season) ([]domain.DriverStanding, error) {
	m.driverCalledWith = season
	return m.driverStandings, m.err
}

func (m *mockF1DataPort) GetConstructorStandings(_ context.Context, season domain.Season) ([]domain.ConstructorStanding, error) {
	m.constructorCalledWith = season
	return m.constructorStandings, m.err
}

type mockCachePort struct {
	store map[string][]byte
}

func newMockCache() *mockCachePort {
	return &mockCachePort{store: make(map[string][]byte)}
}

func (m *mockCachePort) Get(_ context.Context, key string) ([]byte, bool) {
	val, ok := m.store[key]
	return val, ok
}

func (m *mockCachePort) Set(_ context.Context, key string, value []byte) {
	m.store[key] = value
}

// --- Tests ---

func TestGetRaceResult_DelegatesToDataPort(t *testing.T) {
	mock := &mockF1DataPort{
		raceResult: &domain.RaceResult{
			Race: domain.Race{
				Season:   2024,
				Round:    1,
				RaceName: "Bahrain Grand Prix",
			},
			Results: []domain.DriverResult{
				{Position: 1, Driver: domain.Driver{FirstName: "Max", LastName: "Verstappen"}, Points: "25"},
			},
		},
	}

	svc := usecases.NewF1Service(mock, nil)
	result, err := svc.GetRaceResult(context.Background(), 2024, 1)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result.Race.RaceName != "Bahrain Grand Prix" {
		t.Errorf("got race name %q, want %q", result.Race.RaceName, "Bahrain Grand Prix")
	}
	if mock.raceCalledSeason != 2024 {
		t.Errorf("expected season 2024, got %d", mock.raceCalledSeason)
	}
	if mock.raceCalledRound != 1 {
		t.Errorf("expected round 1, got %d", mock.raceCalledRound)
	}
}

func TestGetRaceResult_PropagatesError(t *testing.T) {
	mock := &mockF1DataPort{
		err: errors.New("api unavailable"),
	}

	svc := usecases.NewF1Service(mock, nil)
	_, err := svc.GetRaceResult(context.Background(), 2024, 1)

	if err == nil {
		t.Fatal("expected error, got nil")
	}
	if !errors.Is(err, mock.err) {
		t.Errorf("expected wrapped error containing %q, got %q", mock.err, err)
	}
}

func TestGetRaceResult_UsesCacheOnHit(t *testing.T) {
	mock := &mockF1DataPort{
		raceResult: &domain.RaceResult{
			Race: domain.Race{Season: 2024, Round: 1, RaceName: "Bahrain Grand Prix"},
		},
	}
	cache := newMockCache()

	svc := usecases.NewF1Service(mock, cache)

	// First call: populates cache
	result1, err := svc.GetRaceResult(context.Background(), 2024, 1)
	if err != nil {
		t.Fatalf("first call: unexpected error: %v", err)
	}

	// Swap the mock to return an error — if cache works, we won't hit the port
	mock.err = errors.New("should not be called")

	result2, err := svc.GetRaceResult(context.Background(), 2024, 1)
	if err != nil {
		t.Fatalf("second call (cached): unexpected error: %v", err)
	}

	if result1.Race.RaceName != result2.Race.RaceName {
		t.Errorf("cached result differs: %q vs %q", result1.Race.RaceName, result2.Race.RaceName)
	}
}

func TestGetLatestResult_DelegatesToDataPort(t *testing.T) {
	mock := &mockF1DataPort{
		latestResult: &domain.RaceResult{
			Race: domain.Race{Season: 2024, Round: 5, RaceName: "Chinese Grand Prix"},
		},
	}

	svc := usecases.NewF1Service(mock, nil)
	result, err := svc.GetLatestResult(context.Background())

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !mock.latestCalled {
		t.Error("expected GetLatestRaceResult to be called")
	}
	if result.Race.RaceName != "Chinese Grand Prix" {
		t.Errorf("got %q, want %q", result.Race.RaceName, "Chinese Grand Prix")
	}
}

func TestGetDriverStandings_ReturnsStandings(t *testing.T) {
	mock := &mockF1DataPort{
		driverStandings: []domain.DriverStanding{
			{Position: 1, Points: "200", Driver: domain.Driver{FirstName: "Max", LastName: "Verstappen"}},
			{Position: 2, Points: "150", Driver: domain.Driver{FirstName: "Lando", LastName: "Norris"}},
		},
	}

	svc := usecases.NewF1Service(mock, nil)
	standings, err := svc.GetDriverStandings(context.Background(), 2024)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(standings) != 2 {
		t.Fatalf("expected 2 standings, got %d", len(standings))
	}
	if standings[0].Driver.FullName() != "Max Verstappen" {
		t.Errorf("first driver = %q, want %q", standings[0].Driver.FullName(), "Max Verstappen")
	}
	if mock.driverCalledWith != 2024 {
		t.Errorf("expected season 2024, got %d", mock.driverCalledWith)
	}
}

func TestGetConstructorStandings_ReturnsStandings(t *testing.T) {
	mock := &mockF1DataPort{
		constructorStandings: []domain.ConstructorStanding{
			{Position: 1, Points: "400", Constructor: domain.Constructor{Name: "Red Bull"}},
		},
	}

	svc := usecases.NewF1Service(mock, nil)
	standings, err := svc.GetConstructorStandings(context.Background(), 2024)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(standings) != 1 {
		t.Fatalf("expected 1 standing, got %d", len(standings))
	}
	if standings[0].Constructor.Name != "Red Bull" {
		t.Errorf("constructor = %q, want %q", standings[0].Constructor.Name, "Red Bull")
	}
}

func TestGetFullStandings_CombinesBothStandings(t *testing.T) {
	mock := &mockF1DataPort{
		driverStandings: []domain.DriverStanding{
			{Position: 1, Points: "200", Driver: domain.Driver{FirstName: "Max", LastName: "Verstappen"}},
		},
		constructorStandings: []domain.ConstructorStanding{
			{Position: 1, Points: "400", Constructor: domain.Constructor{Name: "Red Bull"}},
		},
	}

	svc := usecases.NewF1Service(mock, nil)
	resp, err := svc.GetFullStandings(context.Background(), 2024)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if resp.Season != 2024 {
		t.Errorf("season = %d, want 2024", resp.Season)
	}
	if len(resp.DriverStandings) != 1 {
		t.Errorf("expected 1 driver standing, got %d", len(resp.DriverStandings))
	}
	if len(resp.ConstructorStandings) != 1 {
		t.Errorf("expected 1 constructor standing, got %d", len(resp.ConstructorStandings))
	}
}

func TestGetFullStandings_DriverError_Propagates(t *testing.T) {
	mock := &mockF1DataPort{
		err: errors.New("driver standings failed"),
	}

	svc := usecases.NewF1Service(mock, nil)
	_, err := svc.GetFullStandings(context.Background(), 2024)

	if err == nil {
		t.Fatal("expected error, got nil")
	}
}

func TestGetCurrentSchedule_DelegatesToDataPort(t *testing.T) {
	mock := &mockF1DataPort{
		scheduleResult: &domain.SeasonSchedule{
			Season: 2024,
			Races: []domain.Race{
				{Season: 2024, Round: 1, RaceName: "Bahrain Grand Prix"},
				{Season: 2024, Round: 2, RaceName: "Saudi Arabian Grand Prix"},
			},
		},
	}

	svc := usecases.NewF1Service(mock, nil)
	schedule, err := svc.GetCurrentSchedule(context.Background())

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(schedule.Races) != 2 {
		t.Fatalf("expected 2 races, got %d", len(schedule.Races))
	}
	if schedule.Races[0].RaceName != "Bahrain Grand Prix" {
		t.Errorf("first race = %q, want %q", schedule.Races[0].RaceName, "Bahrain Grand Prix")
	}
}

func TestGetCurrentSchedule_PropagatesError(t *testing.T) {
	mock := &mockF1DataPort{
		err: errors.New("schedule unavailable"),
	}

	svc := usecases.NewF1Service(mock, nil)
	_, err := svc.GetCurrentSchedule(context.Background())

	if err == nil {
		t.Fatal("expected error, got nil")
	}
	if !errors.Is(err, mock.err) {
		t.Errorf("expected wrapped error containing %q, got %q", mock.err, err)
	}
}

func TestGetCurrentSchedule_UsesCacheOnHit(t *testing.T) {
	mock := &mockF1DataPort{
		scheduleResult: &domain.SeasonSchedule{
			Season: 2024,
			Races:  []domain.Race{{RaceName: "Bahrain Grand Prix"}},
		},
	}
	cache := newMockCache()
	svc := usecases.NewF1Service(mock, cache)

	// First call populates cache
	_, err := svc.GetCurrentSchedule(context.Background())
	if err != nil {
		t.Fatalf("first call: unexpected error: %v", err)
	}

	// Break mock — cache should prevent hitting it
	mock.err = errors.New("should not be called")

	result, err := svc.GetCurrentSchedule(context.Background())
	if err != nil {
		t.Fatalf("second call (cached): unexpected error: %v", err)
	}
	if result.Races[0].RaceName != "Bahrain Grand Prix" {
		t.Errorf("cached result = %q, want %q", result.Races[0].RaceName, "Bahrain Grand Prix")
	}
}

// --- Multi-error mock (separate error per method) ---

type mockF1DataPortMultiErr struct {
	driverStandings      []domain.DriverStanding
	constructorStandings []domain.ConstructorStanding
	driverErr            error
	constructorErr       error
}

func (m *mockF1DataPortMultiErr) GetSeasonSchedule(_ context.Context, _ domain.Season) (*domain.SeasonSchedule, error) {
	return nil, errors.New("not implemented")
}

func (m *mockF1DataPortMultiErr) GetRaceResult(_ context.Context, _ domain.Season, _ domain.RoundNumber) (*domain.RaceResult, error) {
	return nil, errors.New("not implemented")
}

func (m *mockF1DataPortMultiErr) GetLatestRaceResult(_ context.Context) (*domain.RaceResult, error) {
	return nil, errors.New("not implemented")
}

func (m *mockF1DataPortMultiErr) GetDriverStandings(_ context.Context, _ domain.Season) ([]domain.DriverStanding, error) {
	return m.driverStandings, m.driverErr
}

func (m *mockF1DataPortMultiErr) GetConstructorStandings(_ context.Context, _ domain.Season) ([]domain.ConstructorStanding, error) {
	return m.constructorStandings, m.constructorErr
}

// --- Edge-case tests ---

func TestGetDriverStandings_UsesCacheOnHit(t *testing.T) {
	mock := &mockF1DataPort{
		driverStandings: []domain.DriverStanding{
			{Position: 1, Points: "200", Driver: domain.Driver{FirstName: "Max", LastName: "Verstappen"}},
		},
	}
	cache := newMockCache()
	svc := usecases.NewF1Service(mock, cache)

	// First call populates cache
	result1, err := svc.GetDriverStandings(context.Background(), 2024)
	if err != nil {
		t.Fatalf("first call: unexpected error: %v", err)
	}

	// Break mock — cache should prevent hitting it
	mock.err = errors.New("should not be called")

	result2, err := svc.GetDriverStandings(context.Background(), 2024)
	if err != nil {
		t.Fatalf("second call (cached): unexpected error: %v", err)
	}

	if result1[0].Driver.FullName() != result2[0].Driver.FullName() {
		t.Errorf("cached result differs: %q vs %q", result1[0].Driver.FullName(), result2[0].Driver.FullName())
	}
}

func TestGetConstructorStandings_UsesCacheOnHit(t *testing.T) {
	mock := &mockF1DataPort{
		constructorStandings: []domain.ConstructorStanding{
			{Position: 1, Points: "400", Constructor: domain.Constructor{Name: "Red Bull"}},
		},
	}
	cache := newMockCache()
	svc := usecases.NewF1Service(mock, cache)

	// First call populates cache
	result1, err := svc.GetConstructorStandings(context.Background(), 2024)
	if err != nil {
		t.Fatalf("first call: unexpected error: %v", err)
	}

	// Break mock — cache should prevent hitting it
	mock.err = errors.New("should not be called")

	result2, err := svc.GetConstructorStandings(context.Background(), 2024)
	if err != nil {
		t.Fatalf("second call (cached): unexpected error: %v", err)
	}

	if result1[0].Constructor.Name != result2[0].Constructor.Name {
		t.Errorf("cached result differs: %q vs %q", result1[0].Constructor.Name, result2[0].Constructor.Name)
	}
}

func TestGetFullStandings_ConstructorError_PropagatesEvenWhenDriversSucceed(t *testing.T) {
	mock := &mockF1DataPortMultiErr{
		driverStandings: []domain.DriverStanding{
			{Position: 1, Points: "200", Driver: domain.Driver{FirstName: "Max", LastName: "Verstappen"}},
		},
		driverErr:      nil,
		constructorErr: errors.New("constructor API down"),
	}

	svc := usecases.NewF1Service(mock, nil)
	_, err := svc.GetFullStandings(context.Background(), 2024)

	if err == nil {
		t.Fatal("expected error when constructor standings fail, got nil")
	}
	if !errors.Is(err, mock.constructorErr) {
		t.Errorf("expected wrapped constructor error, got: %v", err)
	}
}

func TestGetLatestResult_UsesCacheOnHit(t *testing.T) {
	mock := &mockF1DataPort{
		latestResult: &domain.RaceResult{
			Race: domain.Race{Season: 2024, Round: 5, RaceName: "Chinese Grand Prix"},
		},
	}
	cache := newMockCache()
	svc := usecases.NewF1Service(mock, cache)

	// First call populates cache
	result1, err := svc.GetLatestResult(context.Background())
	if err != nil {
		t.Fatalf("first call: unexpected error: %v", err)
	}

	// Break mock — cache should prevent hitting it
	mock.err = errors.New("should not be called")

	result2, err := svc.GetLatestResult(context.Background())
	if err != nil {
		t.Fatalf("second call (cached): unexpected error: %v", err)
	}

	if result1.Race.RaceName != result2.Race.RaceName {
		t.Errorf("cached result differs: %q vs %q", result1.Race.RaceName, result2.Race.RaceName)
	}
}

func TestGetFullStandings_UsesCacheOnHit(t *testing.T) {
	mock := &mockF1DataPort{
		driverStandings: []domain.DriverStanding{
			{Position: 1, Points: "200", Driver: domain.Driver{FirstName: "Max", LastName: "Verstappen"}},
		},
		constructorStandings: []domain.ConstructorStanding{
			{Position: 1, Points: "400", Constructor: domain.Constructor{Name: "Red Bull"}},
		},
	}
	cache := newMockCache()
	svc := usecases.NewF1Service(mock, cache)

	// First call populates cache
	result1, err := svc.GetFullStandings(context.Background(), 2024)
	if err != nil {
		t.Fatalf("first call: unexpected error: %v", err)
	}

	// Break mock — cache should prevent hitting it
	mock.err = errors.New("should not be called")

	result2, err := svc.GetFullStandings(context.Background(), 2024)
	if err != nil {
		t.Fatalf("second call (cached): unexpected error: %v", err)
	}

	if result1.Season != result2.Season {
		t.Errorf("cached season differs: %d vs %d", result1.Season, result2.Season)
	}
	if len(result1.DriverStandings) != len(result2.DriverStandings) {
		t.Errorf("cached driver standings count differs: %d vs %d", len(result1.DriverStandings), len(result2.DriverStandings))
	}
	if len(result1.ConstructorStandings) != len(result2.ConstructorStandings) {
		t.Errorf("cached constructor standings count differs: %d vs %d", len(result1.ConstructorStandings), len(result2.ConstructorStandings))
	}
}

func TestNewF1Service_NilCache_WorksFine(t *testing.T) {
	mock := &mockF1DataPort{
		latestResult: &domain.RaceResult{
			Race: domain.Race{RaceName: "Test GP"},
		},
	}

	// nil cache should not panic
	svc := usecases.NewF1Service(mock, nil)
	result, err := svc.GetLatestResult(context.Background())

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result.Race.RaceName != "Test GP" {
		t.Errorf("got %q, want %q", result.Race.RaceName, "Test GP")
	}
}
