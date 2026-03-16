package primary

import (
	"context"
	"embed"
	"fmt"
	"html/template"
	"log"
	"net/http"
	"sort"
	"strconv"
	"time"

	"hex-f1/src/core/domain"
	"hex-f1/src/core/ports"
)

//go:embed templates/*.html
var templateFS embed.FS

// Compile-time interface check.
var _ ports.IHTTPServerPort = (*HTTPAdapter)(nil)

// HTTPAdapter serves the HTMX-powered F1 dashboard.
// It implements ports.IHTTPServerPort via Start/Stop.
type HTTPAdapter struct {
	f1        ports.IF1QueryPort
	server    *http.Server
	templates *template.Template
}

// templateData wraps page content for the layout renderer.
type templateData struct {
	Title   string
	Content template.HTML
	IsHTMX  bool
}

// NewHTTPAdapter creates an HTTPAdapter with parsed embedded templates.
func NewHTTPAdapter(f1 ports.IF1QueryPort) *HTTPAdapter {
	funcMap := template.FuncMap{
		"int": func(p domain.Position) int { return int(p) },
		"seq": func(n int) []int {
			s := make([]int, n)
			for i := range s {
				s[i] = i
			}
			return s
		},
		"sub": func(a, b int) int { return a - b },
		"add": func(a, b int) int { return a + b },
		"mul": func(a, b float64) float64 { return a * b },
		"div": func(a, b float64) float64 {
			if b == 0 {
				return 0
			}
			return a / b
		},
		"toFloat": func(s string) float64 {
			f, _ := strconv.ParseFloat(s, 64)
			return f
		},
		"currentYear": func() int { return time.Now().Year() },
		"isPast": func(dateStr string) bool {
			t, err := time.Parse("2006-01-02", dateStr)
			if err != nil {
				return false
			}
			return t.Before(time.Now())
		},
		"formatDate": func(dateStr string) string {
			t, err := time.Parse("2006-01-02", dateStr)
			if err != nil {
				return dateStr
			}
			return t.Format("Jan 2, 2006")
		},
		"positionSuffix": PositionSuffix,
		"podiumColor": func(p domain.Position) string {
			switch int(p) {
			case 1:
				return "text-yellow-400"
			case 2:
				return "text-gray-300"
			case 3:
				return "text-amber-600"
			default:
				return "text-white"
			}
		},
		"podiumBg": func(p domain.Position) string {
			switch int(p) {
			case 1:
				return "bg-yellow-400/10 border-yellow-400/30"
			case 2:
				return "bg-gray-300/10 border-gray-300/30"
			case 3:
				return "bg-amber-600/10 border-amber-600/30"
			default:
				return "bg-gray-800 border-gray-700"
			}
		},
		"hasFastestLap": func(dr domain.DriverResult) bool {
			return dr.FastestLap != nil && dr.FastestLap.Rank == "1"
		},
		"printf": fmt.Sprintf,
		"teamColor":     TeamColor,
		"teamColorDark": TeamColorDark,
		"driverHash":    DriverHash,
		"mod": func(a, b int) int {
			if b == 0 {
				return 0
			}
			return a % b
		},
		"hashBit": func(hash, bit int) bool {
			return (hash>>uint(bit))&1 == 1
		},
	}

	tmpl, err := template.New("").Funcs(funcMap).ParseFS(templateFS, "templates/*.html")
	if err != nil {
		log.Fatalf("failed to parse templates: %v", err)
	}

	return &HTTPAdapter{
		f1:        f1,
		templates: tmpl,
	}
}

// Handler returns the HTTP handler with all routes registered.
// Exposed for testing — allows use with httptest without starting a real server.
func (h *HTTPAdapter) Handler() http.Handler {
	mux := http.NewServeMux()

	mux.HandleFunc("GET /", h.handleHome)
	mux.HandleFunc("GET /schedule", h.handleSchedule)
	mux.HandleFunc("GET /calendar", h.handleCalendar)
	mux.HandleFunc("GET /results/{season}/{round}", h.handleResults)
	mux.HandleFunc("GET /drivers", h.handleDrivers)
	mux.HandleFunc("GET /constructors", h.handleConstructors)
	mux.HandleFunc("GET /leaderboard", h.handleLeaderboard)

	return mux
}

// Start begins listening on the given address.
func (h *HTTPAdapter) Start(addr string) error {
	h.server = &http.Server{
		Addr:              addr,
		Handler:           h.Handler(),
		ReadHeaderTimeout: 10 * time.Second,
	}

	log.Printf("F1 Dashboard listening on %s", addr)
	if err := h.server.ListenAndServe(); err != nil && err != http.ErrServerClosed {
		return fmt.Errorf("http server: %w", err)
	}
	return nil
}

// Stop gracefully shuts down the server.
func (h *HTTPAdapter) Stop(ctx context.Context) error {
	if h.server == nil {
		return nil
	}
	return h.server.Shutdown(ctx)
}

// isHTMX returns true when the request was made by HTMX.
func isHTMX(r *http.Request) bool {
	return r.Header.Get("HX-Request") == "true"
}

// render executes a partial template; if not HTMX it wraps in the layout.
func (h *HTTPAdapter) render(w http.ResponseWriter, r *http.Request, partial string, title string, data any) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")

	if isHTMX(r) {
		if err := h.templates.ExecuteTemplate(w, partial, data); err != nil {
			log.Printf("template error (%s): %v", partial, err)
			http.Error(w, "Template rendering error", http.StatusInternalServerError)
		}
		return
	}

	// Full page: render partial into a buffer, then wrap in layout.
	buf := &bytesBuffer{}
	if err := h.templates.ExecuteTemplate(buf, partial, data); err != nil {
		log.Printf("template error (%s): %v", partial, err)
		http.Error(w, "Template rendering error", http.StatusInternalServerError)
		return
	}

	layoutData := templateData{
		Title:   title,
		Content: template.HTML(buf.String()),
		IsHTMX:  false,
	}
	if err := h.templates.ExecuteTemplate(w, "layout.html", layoutData); err != nil {
		log.Printf("layout template error: %v", err)
		http.Error(w, "Template rendering error", http.StatusInternalServerError)
	}
}

// --- Handlers ---

func (h *HTTPAdapter) handleHome(w http.ResponseWriter, r *http.Request) {
	// Redirect non-root paths to home (catch-all for GET /)
	if r.URL.Path != "/" {
		http.NotFound(w, r)
		return
	}

	ctx := r.Context()

	latest, err := h.f1.GetLatestResult(ctx)
	if err != nil {
		log.Printf("home: get latest result: %v", err)
		latest = nil
	}

	season := domain.Season(time.Now().Year())
	drivers, err := h.f1.GetDriverStandings(ctx, season)
	if err != nil {
		log.Printf("home: get driver standings: %v", err)
	}

	type homeData struct {
		Latest          *domain.RaceResult
		TopDrivers      []domain.DriverStanding
		CurrentSeason   domain.Season
	}

	topDrivers := drivers
	if len(topDrivers) > 5 {
		topDrivers = topDrivers[:5]
	}

	data := homeData{
		Latest:        latest,
		TopDrivers:    topDrivers,
		CurrentSeason: season,
	}

	h.render(w, r, "home.html", "F1 Race Stats", data)
}

func (h *HTTPAdapter) handleSchedule(w http.ResponseWriter, r *http.Request) {
	ctx := r.Context()

	schedule, err := h.f1.GetCurrentSchedule(ctx)
	if err != nil {
		log.Printf("schedule: %v", err)
		http.Error(w, "Failed to load schedule", http.StatusInternalServerError)
		return
	}

	h.render(w, r, "schedule.html", "Season Schedule", schedule)
}

// calendarMonth holds one month's calendar data for rendering.
type calendarMonth struct {
	Name        string
	Year        int
	MonthNum    int
	DaysInMonth int
	Offset      int // weekday of 1st: Monday=0 .. Sunday=6
	Races       []domain.Race
}

// calendarData is the top-level template data for the calendar page.
type calendarData struct {
	Season      domain.Season
	FutureCount int
	Months      []calendarMonth
	NextRace    *domain.Race
	RaceMap     map[string]*domain.Race // "YYYY-MM-DD" → Race
	Year        int
	MonthNum    int
	DaysInMonth int
}

func (h *HTTPAdapter) handleCalendar(w http.ResponseWriter, r *http.Request) {
	ctx := r.Context()

	schedule, err := h.f1.GetCurrentSchedule(ctx)
	if err != nil {
		log.Printf("calendar: %v", err)
		http.Error(w, "Failed to load schedule", http.StatusInternalServerError)
		return
	}

	now := time.Now()

	// Build a date→race lookup and find the next upcoming race.
	raceMap := make(map[string]*domain.Race, len(schedule.Races))
	var nextRace *domain.Race
	futureCount := 0

	for i := range schedule.Races {
		race := &schedule.Races[i]
		raceMap[race.Date] = race
		t, err := time.Parse("2006-01-02", race.Date)
		if err == nil && !t.Before(now) {
			futureCount++
			if nextRace == nil {
				nextRace = race
			}
		}
	}

	// Group races by month.
	monthRaces := make(map[time.Month][]domain.Race)
	for _, race := range schedule.Races {
		t, err := time.Parse("2006-01-02", race.Date)
		if err != nil {
			continue
		}
		monthRaces[t.Month()] = append(monthRaces[t.Month()], race)
	}

	// Build sorted month list (only months that have races).
	var months []calendarMonth
	sortedMonthNums := make([]int, 0, len(monthRaces))
	for m := range monthRaces {
		sortedMonthNums = append(sortedMonthNums, int(m))
	}
	sort.Ints(sortedMonthNums)

	year := int(schedule.Season)
	for _, mNum := range sortedMonthNums {
		m := time.Month(mNum)
		first := time.Date(year, m, 1, 0, 0, 0, 0, time.UTC)
		// Monday=0 offset
		wd := int(first.Weekday())
		if wd == 0 {
			wd = 7 // Sunday
		}
		offset := wd - 1 // Monday=0

		daysInMonth := time.Date(year, m+1, 0, 0, 0, 0, 0, time.UTC).Day()

		months = append(months, calendarMonth{
			Name:        m.String(),
			Year:        year,
			MonthNum:    mNum,
			DaysInMonth: daysInMonth,
			Offset:      offset,
			Races:       monthRaces[m],
		})
	}

	data := calendarData{
		Season:      schedule.Season,
		FutureCount: futureCount,
		Months:      months,
		NextRace:    nextRace,
		RaceMap:     raceMap,
		Year:        year,
	}

	h.render(w, r, "calendar.html", "Race Calendar", data)
}

func (h *HTTPAdapter) handleResults(w http.ResponseWriter, r *http.Request) {
	ctx := r.Context()

	seasonStr := r.PathValue("season")
	roundStr := r.PathValue("round")

	seasonInt, err := strconv.Atoi(seasonStr)
	if err != nil {
		http.Error(w, "Invalid season", http.StatusBadRequest)
		return
	}

	roundInt, err := strconv.Atoi(roundStr)
	if err != nil {
		http.Error(w, "Invalid round", http.StatusBadRequest)
		return
	}

	result, err := h.f1.GetRaceResult(ctx, domain.Season(seasonInt), domain.RoundNumber(roundInt))
	if err != nil {
		log.Printf("results: %v", err)
		http.Error(w, "Failed to load race result", http.StatusInternalServerError)
		return
	}

	title := fmt.Sprintf("%s Results", result.Race.RaceName)
	h.render(w, r, "results.html", title, result)
}

func (h *HTTPAdapter) handleDrivers(w http.ResponseWriter, r *http.Request) {
	ctx := r.Context()
	season := domain.Season(time.Now().Year())

	standings, err := h.f1.GetDriverStandings(ctx, season)
	if err != nil {
		log.Printf("drivers: %v", err)
		http.Error(w, "Failed to load driver standings", http.StatusInternalServerError)
		return
	}

	type driverData struct {
		Season    domain.Season
		Standings []domain.DriverStanding
		MaxPoints float64
	}

	maxPts := 0.0
	if len(standings) > 0 {
		maxPts, _ = strconv.ParseFloat(standings[0].Points, 64)
	}

	data := driverData{
		Season:    season,
		Standings: standings,
		MaxPoints: maxPts,
	}

	h.render(w, r, "drivers.html", "Driver Standings", data)
}

func (h *HTTPAdapter) handleConstructors(w http.ResponseWriter, r *http.Request) {
	ctx := r.Context()
	season := domain.Season(time.Now().Year())

	standings, err := h.f1.GetConstructorStandings(ctx, season)
	if err != nil {
		log.Printf("constructors: %v", err)
		http.Error(w, "Failed to load constructor standings", http.StatusInternalServerError)
		return
	}

	type constructorData struct {
		Season    domain.Season
		Standings []domain.ConstructorStanding
		MaxPoints float64
	}

	maxPts := 0.0
	if len(standings) > 0 {
		maxPts, _ = strconv.ParseFloat(standings[0].Points, 64)
	}

	data := constructorData{
		Season:    season,
		Standings: standings,
		MaxPoints: maxPts,
	}

	h.render(w, r, "constructors.html", "Constructor Standings", data)
}

func (h *HTTPAdapter) handleLeaderboard(w http.ResponseWriter, r *http.Request) {
	ctx := r.Context()
	season := domain.Season(2025)

	drivers, err := h.f1.GetDriverStandings(ctx, season)
	if err != nil {
		log.Printf("leaderboard: driver standings: %v", err)
		http.Error(w, "Failed to load leaderboard", http.StatusInternalServerError)
		return
	}

	constructors, err := h.f1.GetConstructorStandings(ctx, season)
	if err != nil {
		log.Printf("leaderboard: constructor standings: %v", err)
		http.Error(w, "Failed to load leaderboard", http.StatusInternalServerError)
		return
	}

	driverMax := 0.0
	if len(drivers) > 0 {
		driverMax, _ = strconv.ParseFloat(drivers[0].Points, 64)
	}
	constructorMax := 0.0
	if len(constructors) > 0 {
		constructorMax, _ = strconv.ParseFloat(constructors[0].Points, 64)
	}

	type leaderboardData struct {
		Season               domain.Season
		DriverStandings      []domain.DriverStanding
		ConstructorStandings []domain.ConstructorStanding
		DriverMaxPoints      float64
		ConstructorMaxPoints float64
	}

	data := leaderboardData{
		Season:               season,
		DriverStandings:      drivers,
		ConstructorStandings: constructors,
		DriverMaxPoints:      driverMax,
		ConstructorMaxPoints: constructorMax,
	}

	h.render(w, r, "leaderboard.html", "2025 Championship Leaderboard", data)
}

// bytesBuffer is a simple bytes.Buffer wrapper to avoid importing bytes
// in a way that clutters the import block; it satisfies io.Writer.
type bytesBuffer struct {
	data []byte
}

func (b *bytesBuffer) Write(p []byte) (int, error) {
	b.data = append(b.data, p...)
	return len(p), nil
}

func (b *bytesBuffer) String() string {
	return string(b.data)
}

// TeamColor returns the primary team color hex for a given constructor ID.
// Accepts string because Go templates lose type info in variable assignments.
func TeamColor(id string) string {
	colors := map[string]string{
		"red_bull":     "#3671C6",
		"ferrari":      "#E8002D",
		"mercedes":     "#27F4D2",
		"mclaren":      "#FF8000",
		"aston_martin": "#229971",
		"alpine":       "#FF87BC",
		"williams":     "#64C4FF",
		"rb":           "#6692FF",
		"sauber":       "#52E252",
		"haas":         "#B6BABD",
	}
	if c, ok := colors[id]; ok {
		return c
	}
	return "#FFFFFF"
}

// TeamColorDark returns a darker variant of the team color for gradients.
func TeamColorDark(id string) string {
	colors := map[string]string{
		"red_bull":     "#1B3A66",
		"ferrari":      "#7A0018",
		"mercedes":     "#0A5C4E",
		"mclaren":      "#8B4500",
		"aston_martin": "#0F4A37",
		"alpine":       "#8B4A6A",
		"williams":     "#2E6080",
		"rb":           "#333F80",
		"sauber":       "#1F6B1F",
		"haas":         "#5B5D5F",
	}
	if c, ok := colors[id]; ok {
		return c
	}
	return "#666666"
}

// DriverHash produces a simple numeric hash from a driver ID for generating
// unique geometric patterns in SVG avatars.
func DriverHash(id string) int {
	h := 0
	for _, c := range id {
		h = h*31 + int(c)
	}
	if h < 0 {
		h = -h
	}
	return h
}

// PositionSuffix returns a position number with its ordinal suffix (1st, 2nd, 3rd, etc.).
func PositionSuffix(p domain.Position) string {
	n := int(p)
	switch {
	case n%100 == 11, n%100 == 12, n%100 == 13:
		return fmt.Sprintf("%dth", n)
	case n%10 == 1:
		return fmt.Sprintf("%dst", n)
	case n%10 == 2:
		return fmt.Sprintf("%dnd", n)
	case n%10 == 3:
		return fmt.Sprintf("%drd", n)
	default:
		return fmt.Sprintf("%dth", n)
	}
}
