package domain_test

import (
	"testing"

	"hex-f1/src/core/domain"
)

func TestDriver_FullName(t *testing.T) {
	tests := []struct {
		name      string
		driver    domain.Driver
		wantName  string
	}{
		{
			name:     "standard name",
			driver:   domain.Driver{FirstName: "Max", LastName: "Verstappen"},
			wantName: "Max Verstappen",
		},
		{
			name:     "single name parts",
			driver:   domain.Driver{FirstName: "Lewis", LastName: "Hamilton"},
			wantName: "Lewis Hamilton",
		},
		{
			name:     "empty names",
			driver:   domain.Driver{FirstName: "", LastName: ""},
			wantName: " ",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := tt.driver.FullName()
			if got != tt.wantName {
				t.Errorf("FullName() = %q, want %q", got, tt.wantName)
			}
		})
	}
}

func TestRace_DateTime(t *testing.T) {
	tests := []struct {
		name    string
		race    domain.Race
		wantErr bool
		wantY   int
		wantM   int
		wantD   int
	}{
		{
			name: "valid date and time",
			race: domain.Race{
				Date: "2024-03-02",
				Time: "15:00:00Z",
			},
			wantErr: false,
			wantY:   2024,
			wantM:   3,
			wantD:   2,
		},
		{
			name: "invalid date format",
			race: domain.Race{
				Date: "not-a-date",
				Time: "15:00:00Z",
			},
			wantErr: true,
		},
		{
			name: "empty date and time",
			race: domain.Race{
				Date: "",
				Time: "",
			},
			wantErr: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := tt.race.DateTime()
			if (err != nil) != tt.wantErr {
				t.Errorf("DateTime() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if !tt.wantErr {
				if got.Year() != tt.wantY || int(got.Month()) != tt.wantM || got.Day() != tt.wantD {
					t.Errorf("DateTime() = %v, want %d-%02d-%02d", got, tt.wantY, tt.wantM, tt.wantD)
				}
			}
		})
	}
}

func TestValueObjectTypes(t *testing.T) {
	// Verify value object types work as expected with type conversions
	season := domain.Season(2024)
	if int(season) != 2024 {
		t.Errorf("Season conversion failed: got %d", int(season))
	}

	round := domain.RoundNumber(5)
	if int(round) != 5 {
		t.Errorf("RoundNumber conversion failed: got %d", int(round))
	}

	pos := domain.Position(1)
	if int(pos) != 1 {
		t.Errorf("Position conversion failed: got %d", int(pos))
	}

	driverID := domain.DriverID("max_verstappen")
	if string(driverID) != "max_verstappen" {
		t.Errorf("DriverID conversion failed: got %s", string(driverID))
	}

	lapTime := domain.LapTime("1:30.456")
	if string(lapTime) != "1:30.456" {
		t.Errorf("LapTime conversion failed: got %s", string(lapTime))
	}
}
