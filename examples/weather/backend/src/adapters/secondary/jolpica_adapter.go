package secondary

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strconv"
	"time"

	"hex-f1/src/core/domain"
	"hex-f1/src/core/ports"
)

// --- Internal JSON response types (Jolpica API wrappers) ---

type mrDataResponse struct {
	MRData struct {
		RaceTable      *raceTableJSON      `json:"RaceTable"`
		StandingsTable *standingsTableJSON `json:"StandingsTable"`
	} `json:"MRData"`
}

type raceTableJSON struct {
	Season string          `json:"season"`
	Round  string          `json:"round"`
	Races  []raceResultJSON `json:"Races"`
}

type raceResultJSON struct {
	Season   string             `json:"season"`
	Round    string             `json:"round"`
	RaceName string             `json:"raceName"`
	Circuit  domain.Circuit     `json:"Circuit"`
	Date     string             `json:"date"`
	Time     string             `json:"time"`
	Results  []driverResultJSON `json:"Results"`
}

type driverResultJSON struct {
	Position    string             `json:"position"`
	Points      string             `json:"points"`
	Grid        string             `json:"grid"`
	Laps        string             `json:"laps"`
	Status      string             `json:"status"`
	Driver      domain.Driver      `json:"Driver"`
	Constructor domain.Constructor `json:"Constructor"`
	FastestLap  *fastestLapJSON    `json:"FastestLap,omitempty"`
}

type fastestLapJSON struct {
	Rank         string       `json:"rank"`
	Lap          string       `json:"lap"`
	Time         lapTimeJSON  `json:"Time"`
	AverageSpeed avgSpeedJSON `json:"AverageSpeed"`
}

type lapTimeJSON struct {
	Time string `json:"time"`
}

type avgSpeedJSON struct {
	Speed string `json:"speed"`
}

type standingsTableJSON struct {
	Season         string               `json:"season"`
	StandingsLists []standingsListJSON  `json:"StandingsLists"`
}

type standingsListJSON struct {
	Season                string                      `json:"season"`
	Round                 string                      `json:"round"`
	DriverStandings       []driverStandingJSON        `json:"DriverStandings"`
	ConstructorStandings  []constructorStandingJSON   `json:"ConstructorStandings"`
}

type driverStandingJSON struct {
	Position     string               `json:"position"`
	Points       string               `json:"points"`
	Wins         string               `json:"wins"`
	Driver       domain.Driver        `json:"Driver"`
	Constructors []domain.Constructor `json:"Constructors"`
}

type constructorStandingJSON struct {
	Position    string             `json:"position"`
	Points      string             `json:"points"`
	Wins        string             `json:"wins"`
	Constructor domain.Constructor `json:"Constructor"`
}

// --- JolpicaAdapter ---

// JolpicaAdapter implements ports.IF1DataPort using the Jolpica (Ergast) F1 API.
type JolpicaAdapter struct {
	client  *http.Client
	baseURL string
}

// Compile-time interface check.
var _ ports.IF1DataPort = (*JolpicaAdapter)(nil)

// NewJolpicaAdapter creates a JolpicaAdapter with a 10-second HTTP timeout.
func NewJolpicaAdapter() *JolpicaAdapter {
	return &JolpicaAdapter{
		client: &http.Client{
			Timeout: 10 * time.Second,
		},
		baseURL: "https://api.jolpi.ca/ergast/f1",
	}
}

// fetch performs a GET request and decodes the JSON response into target.
func (a *JolpicaAdapter) fetch(ctx context.Context, path string, target any) error {
	url := a.baseURL + path
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return fmt.Errorf("jolpica: creating request for %s: %w", path, err)
	}
	req.Header.Set("Accept", "application/json")

	resp, err := a.client.Do(req)
	if err != nil {
		return fmt.Errorf("jolpica: fetching %s: %w", path, err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("jolpica: %s returned status %d", path, resp.StatusCode)
	}

	if err := json.NewDecoder(resp.Body).Decode(target); err != nil {
		return fmt.Errorf("jolpica: decoding response from %s: %w", path, err)
	}
	return nil
}

// GetSeasonSchedule returns all races for a given season.
func (a *JolpicaAdapter) GetSeasonSchedule(ctx context.Context, season domain.Season) (*domain.SeasonSchedule, error) {
	path := fmt.Sprintf("/%d.json", int(season))

	var data mrDataResponse
	if err := a.fetch(ctx, path, &data); err != nil {
		return nil, err
	}

	if data.MRData.RaceTable == nil {
		return nil, fmt.Errorf("jolpica: no RaceTable in response for season %d", int(season))
	}

	races := make([]domain.Race, 0, len(data.MRData.RaceTable.Races))
	for _, r := range data.MRData.RaceTable.Races {
		race, err := mapRace(r)
		if err != nil {
			return nil, fmt.Errorf("jolpica: mapping race %q: %w", r.RaceName, err)
		}
		races = append(races, race)
	}

	return &domain.SeasonSchedule{
		Season: season,
		Races:  races,
	}, nil
}

// GetRaceResult returns full results for a specific race.
func (a *JolpicaAdapter) GetRaceResult(ctx context.Context, season domain.Season, round domain.RoundNumber) (*domain.RaceResult, error) {
	path := fmt.Sprintf("/%d/%d/results.json", int(season), int(round))

	var data mrDataResponse
	if err := a.fetch(ctx, path, &data); err != nil {
		return nil, err
	}

	if data.MRData.RaceTable == nil || len(data.MRData.RaceTable.Races) == 0 {
		return nil, fmt.Errorf("jolpica: no race results for season %d round %d", int(season), int(round))
	}

	return mapRaceResult(data.MRData.RaceTable.Races[0])
}

// GetLatestRaceResult returns the most recent completed race result.
func (a *JolpicaAdapter) GetLatestRaceResult(ctx context.Context) (*domain.RaceResult, error) {
	path := "/current/last/results.json"

	var data mrDataResponse
	if err := a.fetch(ctx, path, &data); err != nil {
		return nil, err
	}

	if data.MRData.RaceTable == nil || len(data.MRData.RaceTable.Races) == 0 {
		return nil, fmt.Errorf("jolpica: no latest race result available")
	}

	return mapRaceResult(data.MRData.RaceTable.Races[0])
}

// GetDriverStandings returns current driver championship standings.
func (a *JolpicaAdapter) GetDriverStandings(ctx context.Context, season domain.Season) ([]domain.DriverStanding, error) {
	path := fmt.Sprintf("/%d/driverStandings.json", int(season))

	var data mrDataResponse
	if err := a.fetch(ctx, path, &data); err != nil {
		return nil, err
	}

	if data.MRData.StandingsTable == nil || len(data.MRData.StandingsTable.StandingsLists) == 0 {
		return nil, fmt.Errorf("jolpica: no driver standings for season %d", int(season))
	}

	list := data.MRData.StandingsTable.StandingsLists[0]
	standings := make([]domain.DriverStanding, 0, len(list.DriverStandings))
	for _, ds := range list.DriverStandings {
		pos, err := strconv.Atoi(ds.Position)
		if err != nil {
			return nil, fmt.Errorf("jolpica: parsing driver standing position %q: %w", ds.Position, err)
		}
		standings = append(standings, domain.DriverStanding{
			Position:     domain.Position(pos),
			Points:       ds.Points,
			Wins:         ds.Wins,
			Driver:       ds.Driver,
			Constructors: ds.Constructors,
		})
	}

	return standings, nil
}

// GetConstructorStandings returns current constructor championship standings.
func (a *JolpicaAdapter) GetConstructorStandings(ctx context.Context, season domain.Season) ([]domain.ConstructorStanding, error) {
	path := fmt.Sprintf("/%d/constructorStandings.json", int(season))

	var data mrDataResponse
	if err := a.fetch(ctx, path, &data); err != nil {
		return nil, err
	}

	if data.MRData.StandingsTable == nil || len(data.MRData.StandingsTable.StandingsLists) == 0 {
		return nil, fmt.Errorf("jolpica: no constructor standings for season %d", int(season))
	}

	list := data.MRData.StandingsTable.StandingsLists[0]
	standings := make([]domain.ConstructorStanding, 0, len(list.ConstructorStandings))
	for _, cs := range list.ConstructorStandings {
		pos, err := strconv.Atoi(cs.Position)
		if err != nil {
			return nil, fmt.Errorf("jolpica: parsing constructor standing position %q: %w", cs.Position, err)
		}
		standings = append(standings, domain.ConstructorStanding{
			Position:    domain.Position(pos),
			Points:      cs.Points,
			Wins:        cs.Wins,
			Constructor: cs.Constructor,
		})
	}

	return standings, nil
}

// --- Mapping helpers ---

func mapRace(r raceResultJSON) (domain.Race, error) {
	s, err := strconv.Atoi(r.Season)
	if err != nil {
		return domain.Race{}, fmt.Errorf("parsing season %q: %w", r.Season, err)
	}
	rd, err := strconv.Atoi(r.Round)
	if err != nil {
		return domain.Race{}, fmt.Errorf("parsing round %q: %w", r.Round, err)
	}
	return domain.Race{
		Season:   domain.Season(s),
		Round:    domain.RoundNumber(rd),
		RaceName: r.RaceName,
		Circuit:  r.Circuit,
		Date:     r.Date,
		Time:     r.Time,
	}, nil
}

func mapRaceResult(r raceResultJSON) (*domain.RaceResult, error) {
	race, err := mapRace(r)
	if err != nil {
		return nil, err
	}

	results := make([]domain.DriverResult, 0, len(r.Results))
	for _, dr := range r.Results {
		pos, err := strconv.Atoi(dr.Position)
		if err != nil {
			return nil, fmt.Errorf("parsing position %q: %w", dr.Position, err)
		}

		var fl *domain.FastestLap
		if dr.FastestLap != nil {
			fl = &domain.FastestLap{
				Rank:     dr.FastestLap.Rank,
				Lap:      dr.FastestLap.Lap,
				Time:     domain.LapTime(dr.FastestLap.Time.Time),
				AvgSpeed: dr.FastestLap.AverageSpeed.Speed,
			}
		}

		results = append(results, domain.DriverResult{
			Position:    domain.Position(pos),
			Points:      dr.Points,
			Driver:      dr.Driver,
			Constructor: dr.Constructor,
			Grid:        dr.Grid,
			Laps:        dr.Laps,
			Status:      dr.Status,
			FastestLap:  fl,
		})
	}

	return &domain.RaceResult{
		Race:    race,
		Results: results,
	}, nil
}
