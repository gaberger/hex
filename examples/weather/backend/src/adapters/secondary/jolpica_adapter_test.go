package secondary

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"

	"hex-f1/src/core/domain"
)

// newTestAdapter creates a JolpicaAdapter pointing at the given httptest server.
func newTestAdapter(serverURL string) *JolpicaAdapter {
	a := NewJolpicaAdapter()
	a.baseURL = serverURL
	return a
}

func TestJolpicaAdapter_GetSeasonSchedule_Success(t *testing.T) {
	body := `{
		"MRData": {
			"RaceTable": {
				"season": "2024",
				"Races": [
					{
						"season": "2024",
						"round": "1",
						"raceName": "Bahrain Grand Prix",
						"Circuit": {
							"circuitId": "bahrain",
							"circuitName": "Bahrain International Circuit",
							"Location": {"locality": "Sakhir", "country": "Bahrain", "lat": "26.0325", "long": "50.5106"}
						},
						"date": "2024-03-02",
						"time": "15:00:00Z"
					},
					{
						"season": "2024",
						"round": "2",
						"raceName": "Saudi Arabian Grand Prix",
						"Circuit": {
							"circuitId": "jeddah",
							"circuitName": "Jeddah Corniche Circuit",
							"Location": {"locality": "Jeddah", "country": "Saudi Arabia", "lat": "21.6319", "long": "39.1044"}
						},
						"date": "2024-03-09",
						"time": "17:00:00Z"
					}
				]
			}
		}
	}`

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/2024.json" {
			t.Errorf("unexpected path: %s", r.URL.Path)
		}
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(body))
	}))
	defer srv.Close()

	adapter := newTestAdapter(srv.URL)
	schedule, err := adapter.GetSeasonSchedule(context.Background(), domain.Season(2024))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if schedule.Season != domain.Season(2024) {
		t.Errorf("expected season 2024, got %d", int(schedule.Season))
	}
	if len(schedule.Races) != 2 {
		t.Fatalf("expected 2 races, got %d", len(schedule.Races))
	}
	if schedule.Races[0].RaceName != "Bahrain Grand Prix" {
		t.Errorf("expected Bahrain Grand Prix, got %s", schedule.Races[0].RaceName)
	}
	if schedule.Races[0].Round != domain.RoundNumber(1) {
		t.Errorf("expected round 1, got %d", int(schedule.Races[0].Round))
	}
	if schedule.Races[1].Circuit.ID != domain.CircuitID("jeddah") {
		t.Errorf("expected circuit jeddah, got %s", string(schedule.Races[1].Circuit.ID))
	}
}

func TestJolpicaAdapter_GetSeasonSchedule_NilRaceTable(t *testing.T) {
	body := `{"MRData": {}}`

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(body))
	}))
	defer srv.Close()

	adapter := newTestAdapter(srv.URL)
	_, err := adapter.GetSeasonSchedule(context.Background(), domain.Season(2024))
	if err == nil {
		t.Fatal("expected error for nil RaceTable, got nil")
	}
}

func TestJolpicaAdapter_GetRaceResult_Success(t *testing.T) {
	body := `{
		"MRData": {
			"RaceTable": {
				"season": "2024",
				"round": "1",
				"Races": [
					{
						"season": "2024",
						"round": "1",
						"raceName": "Bahrain Grand Prix",
						"Circuit": {
							"circuitId": "bahrain",
							"circuitName": "Bahrain International Circuit",
							"Location": {"locality": "Sakhir", "country": "Bahrain", "lat": "26.0325", "long": "50.5106"}
						},
						"date": "2024-03-02",
						"time": "15:00:00Z",
						"Results": [
							{
								"position": "1",
								"points": "25",
								"grid": "1",
								"laps": "57",
								"status": "Finished",
								"Driver": {
									"driverId": "max_verstappen",
									"permanentNumber": "1",
									"code": "VER",
									"givenName": "Max",
									"familyName": "Verstappen",
									"dateOfBirth": "1997-09-30",
									"nationality": "Dutch"
								},
								"Constructor": {
									"constructorId": "red_bull",
									"name": "Red Bull",
									"nationality": "Austrian"
								},
								"FastestLap": {
									"rank": "1",
									"lap": "44",
									"Time": {"time": "1:32.608"},
									"AverageSpeed": {"speed": "206.018"}
								}
							},
							{
								"position": "2",
								"points": "18",
								"grid": "3",
								"laps": "57",
								"status": "Finished",
								"Driver": {
									"driverId": "perez",
									"permanentNumber": "11",
									"code": "PER",
									"givenName": "Sergio",
									"familyName": "Perez",
									"dateOfBirth": "1990-01-26",
									"nationality": "Mexican"
								},
								"Constructor": {
									"constructorId": "red_bull",
									"name": "Red Bull",
									"nationality": "Austrian"
								}
							}
						]
					}
				]
			}
		}
	}`

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/2024/1/results.json" {
			t.Errorf("unexpected path: %s", r.URL.Path)
		}
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(body))
	}))
	defer srv.Close()

	adapter := newTestAdapter(srv.URL)
	result, err := adapter.GetRaceResult(context.Background(), domain.Season(2024), domain.RoundNumber(1))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if result.Race.RaceName != "Bahrain Grand Prix" {
		t.Errorf("expected Bahrain Grand Prix, got %s", result.Race.RaceName)
	}
	if len(result.Results) != 2 {
		t.Fatalf("expected 2 results, got %d", len(result.Results))
	}

	// First driver with fastest lap
	dr := result.Results[0]
	if dr.Position != domain.Position(1) {
		t.Errorf("expected position 1, got %d", int(dr.Position))
	}
	if dr.Driver.ID != domain.DriverID("max_verstappen") {
		t.Errorf("expected max_verstappen, got %s", string(dr.Driver.ID))
	}
	if dr.FastestLap == nil {
		t.Fatal("expected FastestLap to be non-nil")
	}
	if dr.FastestLap.Time != domain.LapTime("1:32.608") {
		t.Errorf("expected lap time 1:32.608, got %s", string(dr.FastestLap.Time))
	}
	if dr.FastestLap.AvgSpeed != "206.018" {
		t.Errorf("expected avg speed 206.018, got %s", dr.FastestLap.AvgSpeed)
	}

	// Second driver without fastest lap
	if result.Results[1].FastestLap != nil {
		t.Error("expected FastestLap to be nil for second driver")
	}
}

func TestJolpicaAdapter_GetRaceResult_NoRaces(t *testing.T) {
	body := `{
		"MRData": {
			"RaceTable": {
				"season": "2024",
				"round": "99",
				"Races": []
			}
		}
	}`

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(body))
	}))
	defer srv.Close()

	adapter := newTestAdapter(srv.URL)
	_, err := adapter.GetRaceResult(context.Background(), domain.Season(2024), domain.RoundNumber(99))
	if err == nil {
		t.Fatal("expected error for empty Races, got nil")
	}
}

func TestJolpicaAdapter_GetLatestRaceResult_Success(t *testing.T) {
	body := `{
		"MRData": {
			"RaceTable": {
				"season": "2024",
				"round": "5",
				"Races": [
					{
						"season": "2024",
						"round": "5",
						"raceName": "Chinese Grand Prix",
						"Circuit": {
							"circuitId": "shanghai",
							"circuitName": "Shanghai International Circuit",
							"Location": {"locality": "Shanghai", "country": "China", "lat": "31.3389", "long": "121.2198"}
						},
						"date": "2024-04-21",
						"time": "07:00:00Z",
						"Results": [
							{
								"position": "1",
								"points": "25",
								"grid": "1",
								"laps": "56",
								"status": "Finished",
								"Driver": {
									"driverId": "max_verstappen",
									"permanentNumber": "1",
									"code": "VER",
									"givenName": "Max",
									"familyName": "Verstappen",
									"dateOfBirth": "1997-09-30",
									"nationality": "Dutch"
								},
								"Constructor": {
									"constructorId": "red_bull",
									"name": "Red Bull",
									"nationality": "Austrian"
								}
							}
						]
					}
				]
			}
		}
	}`

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/current/last/results.json" {
			t.Errorf("expected /current/last/results.json, got %s", r.URL.Path)
		}
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(body))
	}))
	defer srv.Close()

	adapter := newTestAdapter(srv.URL)
	result, err := adapter.GetLatestRaceResult(context.Background())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if result.Race.RaceName != "Chinese Grand Prix" {
		t.Errorf("expected Chinese Grand Prix, got %s", result.Race.RaceName)
	}
	if result.Race.Round != domain.RoundNumber(5) {
		t.Errorf("expected round 5, got %d", int(result.Race.Round))
	}
}

func TestJolpicaAdapter_GetDriverStandings_Success(t *testing.T) {
	body := `{
		"MRData": {
			"StandingsTable": {
				"season": "2024",
				"StandingsLists": [
					{
						"season": "2024",
						"round": "5",
						"DriverStandings": [
							{
								"position": "1",
								"points": "136",
								"wins": "4",
								"Driver": {
									"driverId": "max_verstappen",
									"permanentNumber": "1",
									"code": "VER",
									"givenName": "Max",
									"familyName": "Verstappen",
									"dateOfBirth": "1997-09-30",
									"nationality": "Dutch"
								},
								"Constructors": [
									{
										"constructorId": "red_bull",
										"name": "Red Bull",
										"nationality": "Austrian"
									}
								]
							},
							{
								"position": "2",
								"points": "91",
								"wins": "1",
								"Driver": {
									"driverId": "perez",
									"permanentNumber": "11",
									"code": "PER",
									"givenName": "Sergio",
									"familyName": "Perez",
									"dateOfBirth": "1990-01-26",
									"nationality": "Mexican"
								},
								"Constructors": [
									{
										"constructorId": "red_bull",
										"name": "Red Bull",
										"nationality": "Austrian"
									}
								]
							}
						]
					}
				]
			}
		}
	}`

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/2024/driverStandings.json" {
			t.Errorf("unexpected path: %s", r.URL.Path)
		}
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(body))
	}))
	defer srv.Close()

	adapter := newTestAdapter(srv.URL)
	standings, err := adapter.GetDriverStandings(context.Background(), domain.Season(2024))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(standings) != 2 {
		t.Fatalf("expected 2 standings, got %d", len(standings))
	}
	if standings[0].Position != domain.Position(1) {
		t.Errorf("expected position 1, got %d", int(standings[0].Position))
	}
	if standings[0].Points != "136" {
		t.Errorf("expected 136 points, got %s", standings[0].Points)
	}
	if standings[0].Driver.Code != "VER" {
		t.Errorf("expected VER, got %s", standings[0].Driver.Code)
	}
	if standings[1].Position != domain.Position(2) {
		t.Errorf("expected position 2, got %d", int(standings[1].Position))
	}
}

func TestJolpicaAdapter_GetConstructorStandings_Success(t *testing.T) {
	body := `{
		"MRData": {
			"StandingsTable": {
				"season": "2024",
				"StandingsLists": [
					{
						"season": "2024",
						"round": "5",
						"ConstructorStandings": [
							{
								"position": "1",
								"points": "227",
								"wins": "5",
								"Constructor": {
									"constructorId": "red_bull",
									"name": "Red Bull",
									"nationality": "Austrian"
								}
							},
							{
								"position": "2",
								"points": "120",
								"wins": "0",
								"Constructor": {
									"constructorId": "ferrari",
									"name": "Ferrari",
									"nationality": "Italian"
								}
							}
						]
					}
				]
			}
		}
	}`

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/2024/constructorStandings.json" {
			t.Errorf("unexpected path: %s", r.URL.Path)
		}
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(body))
	}))
	defer srv.Close()

	adapter := newTestAdapter(srv.URL)
	standings, err := adapter.GetConstructorStandings(context.Background(), domain.Season(2024))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(standings) != 2 {
		t.Fatalf("expected 2 standings, got %d", len(standings))
	}
	if standings[0].Position != domain.Position(1) {
		t.Errorf("expected position 1, got %d", int(standings[0].Position))
	}
	if standings[0].Constructor.Name != "Red Bull" {
		t.Errorf("expected Red Bull, got %s", standings[0].Constructor.Name)
	}
	if standings[1].Constructor.ID != domain.ConstructorID("ferrari") {
		t.Errorf("expected ferrari, got %s", string(standings[1].Constructor.ID))
	}
	if standings[1].Wins != "0" {
		t.Errorf("expected 0 wins, got %s", standings[1].Wins)
	}
}

func TestJolpicaAdapter_FetchErrorOnNon200(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusInternalServerError)
	}))
	defer srv.Close()

	adapter := newTestAdapter(srv.URL)
	_, err := adapter.GetSeasonSchedule(context.Background(), domain.Season(2024))
	if err == nil {
		t.Fatal("expected error for HTTP 500, got nil")
	}
}

func TestJolpicaAdapter_FetchErrorOnBadJSON(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{not valid json`))
	}))
	defer srv.Close()

	adapter := newTestAdapter(srv.URL)
	_, err := adapter.GetSeasonSchedule(context.Background(), domain.Season(2024))
	if err == nil {
		t.Fatal("expected error for bad JSON, got nil")
	}
}
