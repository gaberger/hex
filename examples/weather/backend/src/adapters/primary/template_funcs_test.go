package primary_test

import (
	"testing"

	"hex-f1/src/adapters/primary"
	"hex-f1/src/core/domain"
)

func TestPositionSuffix(t *testing.T) {
	tests := []struct {
		pos  domain.Position
		want string
	}{
		{1, "1st"},
		{2, "2nd"},
		{3, "3rd"},
		{4, "4th"},
		{5, "5th"},
		{10, "10th"},
		{11, "11th"}, // special: not 11st
		{12, "12th"}, // special: not 12nd
		{13, "13th"}, // special: not 13rd
		{14, "14th"},
		{20, "20th"},
		{21, "21st"},
		{22, "22nd"},
		{23, "23rd"},
		{101, "101st"},
		{111, "111th"}, // special
		{112, "112th"}, // special
		{113, "113th"}, // special
	}

	for _, tt := range tests {
		t.Run(tt.want, func(t *testing.T) {
			got := primary.PositionSuffix(tt.pos)
			if got != tt.want {
				t.Errorf("PositionSuffix(%d) = %q, want %q", tt.pos, got, tt.want)
			}
		})
	}
}

func TestTeamColor(t *testing.T) {
	tests := []struct {
		id   string
		want string
	}{
		{"red_bull", "#3671C6"},
		{"ferrari", "#E8002D"},
		{"mercedes", "#27F4D2"},
		{"mclaren", "#FF8000"},
		{"aston_martin", "#229971"},
		{"alpine", "#FF87BC"},
		{"williams", "#64C4FF"},
		{"rb", "#6692FF"},
		{"sauber", "#52E252"},
		{"haas", "#B6BABD"},
		{"unknown_team", "#FFFFFF"},
		{"", "#FFFFFF"},
	}

	for _, tt := range tests {
		t.Run(tt.id, func(t *testing.T) {
			got := primary.TeamColor(tt.id)
			if got != tt.want {
				t.Errorf("TeamColor(%q) = %q, want %q", tt.id, got, tt.want)
			}
		})
	}
}

func TestTeamColorDark(t *testing.T) {
	tests := []struct {
		id   string
		want string
	}{
		{"red_bull", "#1B3A66"},
		{"ferrari", "#7A0018"},
		{"mercedes", "#0A5C4E"},
		{"mclaren", "#8B4500"},
		{"aston_martin", "#0F4A37"},
		{"alpine", "#8B4A6A"},
		{"williams", "#2E6080"},
		{"rb", "#333F80"},
		{"sauber", "#1F6B1F"},
		{"haas", "#5B5D5F"},
		{"unknown_team", "#666666"},
		{"", "#666666"},
	}

	for _, tt := range tests {
		t.Run(tt.id, func(t *testing.T) {
			got := primary.TeamColorDark(tt.id)
			if got != tt.want {
				t.Errorf("TeamColorDark(%q) = %q, want %q", tt.id, got, tt.want)
			}
		})
	}
}

func TestDriverHash(t *testing.T) {
	tests := []struct {
		id string
	}{
		{"verstappen"},
		{"hamilton"},
		{"norris"},
		{""},
	}

	for _, tt := range tests {
		t.Run(tt.id, func(t *testing.T) {
			got := primary.DriverHash(tt.id)
			if got < 0 {
				t.Errorf("DriverHash(%q) = %d, want non-negative", tt.id, got)
			}
		})
	}

	// Deterministic: same input always returns same output.
	hash1 := primary.DriverHash("verstappen")
	hash2 := primary.DriverHash("verstappen")
	if hash1 != hash2 {
		t.Errorf("DriverHash should be deterministic: got %d and %d", hash1, hash2)
	}

	// Different inputs should (very likely) produce different hashes.
	if primary.DriverHash("verstappen") == primary.DriverHash("hamilton") {
		t.Error("DriverHash should produce different values for different inputs")
	}
}
