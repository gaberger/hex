package primary_test

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"hex-f1/src/adapters/primary"
	"hex-f1/src/core/domain"
)

// --- Mock IF1QueryPort (London-school: mock at the port boundary) ---

type mockF1QueryPort struct {
	schedule             *domain.SeasonSchedule
	raceResult           *domain.RaceResult
	latestResult         *domain.RaceResult
	driverStandings      []domain.DriverStanding
	constructorStandings []domain.ConstructorStanding
	err                  error
}

func (m *mockF1QueryPort) GetCurrentSchedule(_ context.Context) (*domain.SeasonSchedule, error) {
	return m.schedule, m.err
}

func (m *mockF1QueryPort) GetRaceResult(_ context.Context, _ domain.Season, _ domain.RoundNumber) (*domain.RaceResult, error) {
	return m.raceResult, m.err
}

func (m *mockF1QueryPort) GetLatestResult(_ context.Context) (*domain.RaceResult, error) {
	return m.latestResult, m.err
}

func (m *mockF1QueryPort) GetDriverStandings(_ context.Context, _ domain.Season) ([]domain.DriverStanding, error) {
	return m.driverStandings, m.err
}

func (m *mockF1QueryPort) GetConstructorStandings(_ context.Context, _ domain.Season) ([]domain.ConstructorStanding, error) {
	return m.constructorStandings, m.err
}

// --- Tests ---

func TestHTTPAdapter_HomeReturnsFullPage(t *testing.T) {
	mock := &mockF1QueryPort{
		latestResult: &domain.RaceResult{
			Race: domain.Race{Season: 2024, Round: 1, RaceName: "Bahrain Grand Prix"},
			Results: []domain.DriverResult{
				{Position: 1, Driver: domain.Driver{FirstName: "Max", LastName: "Verstappen"}, Points: "25"},
			},
		},
		driverStandings: []domain.DriverStanding{
			{Position: 1, Points: "200", Driver: domain.Driver{FirstName: "Max", LastName: "Verstappen"}},
		},
	}

	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}

	body := w.Body.String()
	if !strings.Contains(body, "<html") {
		t.Error("full page request should contain <html> layout wrapper")
	}
}

func TestHTTPAdapter_HomeHTMXReturnsPartial(t *testing.T) {
	mock := &mockF1QueryPort{
		latestResult: &domain.RaceResult{
			Race: domain.Race{Season: 2024, Round: 1, RaceName: "Test GP"},
		},
		driverStandings: []domain.DriverStanding{},
	}

	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/", nil)
	req.Header.Set("HX-Request", "true")
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}

	body := w.Body.String()
	if strings.Contains(body, "<html") {
		t.Error("HTMX request should return partial without <html> wrapper")
	}
}

func TestHTTPAdapter_UnknownPathReturns404(t *testing.T) {
	mock := &mockF1QueryPort{}
	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/nonexistent", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusNotFound {
		t.Errorf("expected 404 for unknown path, got %d", w.Code)
	}
}

func TestHTTPAdapter_ResultsInvalidSeasonReturns400(t *testing.T) {
	mock := &mockF1QueryPort{}
	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/results/notanumber/1", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusBadRequest {
		t.Errorf("expected 400 for invalid season, got %d", w.Code)
	}
}

func TestHTTPAdapter_ResultsInvalidRoundReturns400(t *testing.T) {
	mock := &mockF1QueryPort{}
	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/results/2024/abc", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusBadRequest {
		t.Errorf("expected 400 for invalid round, got %d", w.Code)
	}
}

func TestHTTPAdapter_ScheduleReturns200(t *testing.T) {
	mock := &mockF1QueryPort{
		schedule: &domain.SeasonSchedule{
			Season: 2024,
			Races: []domain.Race{
				{Season: 2024, Round: 1, RaceName: "Bahrain GP", Date: "2024-03-02"},
			},
		},
	}

	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/schedule", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}
}

func TestHTTPAdapter_DriversReturns200(t *testing.T) {
	mock := &mockF1QueryPort{
		driverStandings: []domain.DriverStanding{
			{Position: 1, Points: "200", Driver: domain.Driver{FirstName: "Max", LastName: "Verstappen"}},
		},
	}

	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/drivers", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}
}

func TestHTTPAdapter_ConstructorsReturns200(t *testing.T) {
	mock := &mockF1QueryPort{
		constructorStandings: []domain.ConstructorStanding{
			{Position: 1, Points: "400", Constructor: domain.Constructor{Name: "Red Bull"}},
		},
	}

	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/constructors", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}
}

func TestHTTPAdapter_LeaderboardReturns200(t *testing.T) {
	mock := &mockF1QueryPort{
		driverStandings: []domain.DriverStanding{
			{Position: 1, Points: "110", Wins: "4", Driver: domain.Driver{FirstName: "Max", LastName: "Verstappen"}, Constructors: []domain.Constructor{{Name: "Red Bull"}}},
			{Position: 2, Points: "85", Wins: "2", Driver: domain.Driver{FirstName: "Lando", LastName: "Norris"}, Constructors: []domain.Constructor{{Name: "McLaren"}}},
		},
		constructorStandings: []domain.ConstructorStanding{
			{Position: 1, Points: "190", Wins: "5", Constructor: domain.Constructor{Name: "Red Bull", Nationality: "Austrian"}},
			{Position: 2, Points: "150", Wins: "3", Constructor: domain.Constructor{Name: "McLaren", Nationality: "British"}},
		},
	}

	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/leaderboard", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}

	body := w.Body.String()
	if !strings.Contains(body, "Championship Leaderboard") {
		t.Error("expected leaderboard heading in response")
	}
	if !strings.Contains(body, "Max Verstappen") {
		t.Error("expected driver name in response")
	}
	if !strings.Contains(body, "Red Bull") {
		t.Error("expected constructor name in response")
	}
}

func TestHTTPAdapter_LeaderboardErrorReturns500(t *testing.T) {
	mock := &mockF1QueryPort{
		err: fmt.Errorf("API unavailable"),
	}

	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/leaderboard", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusInternalServerError {
		t.Errorf("expected 500 on error, got %d", w.Code)
	}
}

// --- Smoke test: server starts and responds ---

func TestHTTPAdapter_StartAndStop(t *testing.T) {
	mock := &mockF1QueryPort{
		latestResult: &domain.RaceResult{
			Race: domain.Race{RaceName: "Test GP"},
		},
		driverStandings: []domain.DriverStanding{},
	}

	adapter := primary.NewHTTPAdapter(mock)

	// Find a free port
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("failed to find free port: %v", err)
	}
	addr := listener.Addr().String()
	listener.Close()

	// Start server in background
	errCh := make(chan error, 1)
	go func() {
		errCh <- adapter.Start(addr)
	}()

	// Give server time to start
	time.Sleep(100 * time.Millisecond)

	// Hit the home page
	resp, err := http.Get(fmt.Sprintf("http://%s/", addr))
	if err != nil {
		t.Fatalf("failed to GET /: %v", err)
	}
	resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		t.Errorf("expected 200, got %d", resp.StatusCode)
	}

	// Graceful shutdown
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	if err := adapter.Stop(ctx); err != nil {
		t.Fatalf("shutdown error: %v", err)
	}
}

func TestHTTPAdapter_CalendarReturns200(t *testing.T) {
	mock := &mockF1QueryPort{
		schedule: &domain.SeasonSchedule{
			Season: 2024,
			Races: []domain.Race{
				{Season: 2024, Round: 1, RaceName: "Bahrain GP", Date: "2024-03-02"},
				{Season: 2024, Round: 5, RaceName: "Miami GP", Date: "2024-05-05"},
				{Season: 2024, Round: 10, RaceName: "British GP", Date: "2024-07-07"},
				{Season: 2024, Round: 20, RaceName: "Abu Dhabi GP", Date: "2024-12-08"},
			},
		},
	}

	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/calendar", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}

	body := w.Body.String()
	if !strings.Contains(body, "Bahrain GP") {
		t.Error("expected Bahrain GP in calendar response")
	}
}

func TestHTTPAdapter_ScheduleErrorReturns500(t *testing.T) {
	mock := &mockF1QueryPort{
		err: fmt.Errorf("schedule API error"),
	}

	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/schedule", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusInternalServerError {
		t.Errorf("expected 500 on schedule error, got %d", w.Code)
	}
}

func TestHTTPAdapter_DriversErrorReturns500(t *testing.T) {
	mock := &mockF1QueryPort{
		err: fmt.Errorf("drivers API error"),
	}

	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/drivers", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusInternalServerError {
		t.Errorf("expected 500 on drivers error, got %d", w.Code)
	}
}

func TestHTTPAdapter_ConstructorsErrorReturns500(t *testing.T) {
	mock := &mockF1QueryPort{
		err: fmt.Errorf("constructors API error"),
	}

	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/constructors", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusInternalServerError {
		t.Errorf("expected 500 on constructors error, got %d", w.Code)
	}
}

func TestHTTPAdapter_ResultsSuccessReturns200(t *testing.T) {
	mock := &mockF1QueryPort{
		raceResult: &domain.RaceResult{
			Race: domain.Race{Season: 2024, Round: 1, RaceName: "Bahrain Grand Prix"},
			Results: []domain.DriverResult{
				{Position: 1, Driver: domain.Driver{FirstName: "Max", LastName: "Verstappen"}, Points: "25"},
				{Position: 2, Driver: domain.Driver{FirstName: "Sergio", LastName: "Perez"}, Points: "18"},
			},
		},
	}

	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/results/2024/1", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}

	body := w.Body.String()
	if !strings.Contains(body, "Verstappen") {
		t.Error("expected driver name in results response")
	}
}

func TestHTTPAdapter_ResultsErrorReturns500(t *testing.T) {
	mock := &mockF1QueryPort{
		err: fmt.Errorf("results API error"),
	}

	adapter := primary.NewHTTPAdapter(mock)
	handler := adapter.Handler()

	req := httptest.NewRequest("GET", "/results/2024/1", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, req)

	if w.Code != http.StatusInternalServerError {
		t.Errorf("expected 500 on results error, got %d", w.Code)
	}
}
