package domain

import "time"

// --- Value Objects ---

type Season int

type RoundNumber int

type DriverID string

type ConstructorID string

type CircuitID string

// Position in a race result (1-based)
type Position int

// LapTime represents a formatted lap time string like "1:30.456"
type LapTime string

// --- Entities ---

type Driver struct {
	ID          DriverID `json:"driverId"`
	Number      string   `json:"permanentNumber"`
	Code        string   `json:"code"`
	FirstName   string   `json:"givenName"`
	LastName    string   `json:"familyName"`
	DateOfBirth string   `json:"dateOfBirth"`
	Nationality string   `json:"nationality"`
}

func (d Driver) FullName() string {
	return d.FirstName + " " + d.LastName
}

type Constructor struct {
	ID          ConstructorID `json:"constructorId"`
	Name        string        `json:"name"`
	Nationality string        `json:"nationality"`
}

type Circuit struct {
	ID       CircuitID `json:"circuitId"`
	Name     string    `json:"circuitName"`
	Location Location  `json:"Location"`
}

type Location struct {
	Locality string `json:"locality"`
	Country  string `json:"country"`
	Lat      string `json:"lat"`
	Long     string `json:"long"`
}

type Race struct {
	Season   Season      `json:"season"`
	Round    RoundNumber `json:"round"`
	RaceName string      `json:"raceName"`
	Circuit  Circuit     `json:"Circuit"`
	Date     string      `json:"date"`
	Time     string      `json:"time"`
}

func (r Race) DateTime() (time.Time, error) {
	return time.Parse("2006-01-02T15:04:05Z", r.Date+"T"+r.Time)
}

type RaceResult struct {
	Race    Race           `json:"race"`
	Results []DriverResult `json:"Results"`
}

type DriverResult struct {
	Position    Position    `json:"position"`
	Points      string      `json:"points"`
	Driver      Driver      `json:"Driver"`
	Constructor Constructor `json:"Constructor"`
	Grid        string      `json:"grid"`
	Laps        string      `json:"laps"`
	Status      string      `json:"status"`
	FastestLap  *FastestLap `json:"FastestLap,omitempty"`
}

type FastestLap struct {
	Rank    string  `json:"rank"`
	Lap     string  `json:"lap"`
	Time    LapTime `json:"Time"`
	AvgSpeed string `json:"AverageSpeed"`
}

type DriverStanding struct {
	Position     Position      `json:"position"`
	Points       string        `json:"points"`
	Wins         string        `json:"wins"`
	Driver       Driver        `json:"Driver"`
	Constructors []Constructor `json:"Constructors"`
}

type ConstructorStanding struct {
	Position    Position    `json:"position"`
	Points      string      `json:"points"`
	Wins        string      `json:"wins"`
	Constructor Constructor `json:"Constructor"`
}

// --- Aggregate Responses ---

type SeasonSchedule struct {
	Season Season `json:"season"`
	Races  []Race `json:"Races"`
}

type StandingsResponse struct {
	Season              Season                `json:"season"`
	DriverStandings     []DriverStanding      `json:"driverStandings"`
	ConstructorStandings []ConstructorStanding `json:"constructorStandings"`
}
