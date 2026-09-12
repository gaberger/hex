package gamettt

import (
	"errors"
	"sync"
	"testing"
)

// Measured on this machine, go1.22.2 linux/amd64, 2026-09-11. These are real
// wall-clock times, not node counts:
//
//	go test ./...        0.19 s for the package
//	go test -race ./...  1.76 s for the package
//
// The counts the spec gives are all reproduced here, by the tests themselves:
// 5478 reachable positions, 4520 live, 2358 with a win available, 1052 drawn,
// and 91 BestMove calls as X with 768 as O in test 6.
//
// The suite was checked against four broken players, and each one was caught:
// depth-blind scoring (tests 3 and 7), Full() before Winner() (tests 6 and 8),
// a >= tie-break (test 5), and Move on a value receiver (test 2).

// ---------------------------------------------------------------------------
// Shared helpers. These are the outside truths the tests measure against.
// ---------------------------------------------------------------------------

func mustParse(t *testing.T, s string) Board {
	t.Helper()
	b, err := Parse(s)
	if err != nil {
		t.Fatalf("Parse(%q): unexpected error %v", s, err)
	}
	return b
}

// outcome is an independent oracle. It is a plain min-max with no depth term
// and no negation trick, written differently from the code it judges, so a
// misunderstanding in the search cannot copy itself into the test.
//
// It returns +1 when p wins with perfect play, 0 for a draw, -1 when p loses.
func outcome(b Board, p Player) int {
	if w := b.Winner(); w != Empty {
		if w == p {
			return 1
		}
		return -1
	}
	if b.Full() {
		return 0
	}
	mover := b.Turn()
	best, first := 0, true
	for i := 0; i < 9; i++ {
		if b.cells[i] != Empty {
			continue
		}
		c := b
		c.cells[i] = mover
		r := outcome(c, p)
		switch {
		case first:
			best, first = r, false
		case mover == p && r > best:
			best = r
		case mover != p && r < best:
			best = r
		}
	}
	return best
}

// winningCells lists the cells that win for p at once.
func winningCells(b Board, p Player) []int {
	var out []int
	for i := 0; i < 9; i++ {
		if b.cells[i] != Empty {
			continue
		}
		c := b
		c.cells[i] = p
		if c.Winner() == p {
			out = append(out, i)
		}
	}
	return out
}

var (
	tableOnce sync.Once
	// every position a real game can reach, terminal ones included
	allReachable map[Board]bool
	// reachable positions where the game still runs
	livePositions []Board
	// live positions where the side to move can win at once
	winPositions []Board
	// live positions where the side to move can only draw
	drawPositions []Board
)

func buildTables() {
	tableOnce.Do(func() {
		allReachable = make(map[Board]bool, 6000)
		var walk func(b Board)
		walk = func(b Board) {
			if allReachable[b] {
				return
			}
			allReachable[b] = true
			if b.Winner() != Empty || b.Full() {
				return
			}
			p := b.Turn()
			for i := 0; i < 9; i++ {
				if b.cells[i] != Empty {
					continue
				}
				c := b
				if err := c.Move(i, p); err != nil {
					panic("walk: legal move rejected: " + err.Error())
				}
				walk(c)
			}
		}
		walk(Board{})

		for b := range allReachable {
			if b.Winner() != Empty || b.Full() {
				continue
			}
			livePositions = append(livePositions, b)
			mover := b.Turn()
			if len(winningCells(b, mover)) > 0 {
				winPositions = append(winPositions, b)
			}
			if outcome(b, mover) == 0 {
				drawPositions = append(drawPositions, b)
			}
		}
	})
}

// ---------------------------------------------------------------------------
// Test 1 — a winning row is detected
// ---------------------------------------------------------------------------

func TestWinningLineIsDetected(t *testing.T) {
	cases := []struct {
		board string
		line  [3]int
		want  Player
	}{
		{"XXXOO....", [3]int{0, 1, 2}, X},
		{"OO.XXX...", [3]int{3, 4, 5}, X},
		{"OO....XXX", [3]int{6, 7, 8}, X},
		{"XOOX..X..", [3]int{0, 3, 6}, X},
		{"OXO.X..X.", [3]int{1, 4, 7}, X},
		{"OOX..X..X", [3]int{2, 5, 8}, X},
		{"XOO.X...X", [3]int{0, 4, 8}, X},
		{"OOX.X.X..", [3]int{2, 4, 6}, X},
		{"OOOXX.X..", [3]int{0, 1, 2}, O},
		{"XX.OOOX..", [3]int{3, 4, 5}, O},
		{"XX.X..OOO", [3]int{6, 7, 8}, O},
		{"OXXOX.O..", [3]int{0, 3, 6}, O},
		{"XOXXO..O.", [3]int{1, 4, 7}, O},
		{"XXOX.O..O", [3]int{2, 5, 8}, O},
		{"OXXXO...O", [3]int{0, 4, 8}, O},
		{"XXOXO.O..", [3]int{2, 4, 6}, O},
	}
	if len(cases) != 16 {
		t.Fatalf("want 16 cases, got %d", len(cases))
	}
	for _, tc := range cases {
		b := mustParse(t, tc.board)
		if got := b.Winner(); got != tc.want {
			t.Errorf("%s: Winner()=%d, want %d (line %v)", b, got, tc.want, tc.line)
		}
		// The named line really is the winning one.
		for _, i := range tc.line {
			if b.cells[i] != tc.want {
				t.Errorf("%s: cell %d is %d, want %d", b, i, b.cells[i], tc.want)
			}
		}
	}
}

// ---------------------------------------------------------------------------
// Test 2 — a full board with no winner is a draw
// ---------------------------------------------------------------------------

func TestFullBoardWithNoWinnerIsADraw(t *testing.T) {
	// Built by replaying legal moves, not by Parse, so Move itself is proved
	// able to reach a full board.
	seq := []struct {
		pos int
		p   Player
	}{
		{0, X}, {4, O}, {8, X}, {2, O}, {6, X}, {3, O}, {5, X}, {7, O}, {1, X},
	}
	var b Board
	for n, m := range seq {
		if err := b.Move(m.pos, m.p); err != nil {
			t.Fatalf("move %d (%d by %d): %v; board %s", n, m.pos, m.p, err, b)
		}
	}
	if got := b.Winner(); got != Empty {
		t.Errorf("%s: Winner()=%d, want Empty", b, got)
	}
	if !b.Full() {
		t.Errorf("%s: Full()=false, want true", b)
	}
	// Winner alone cannot tell a draw from an unfinished game. Both hold here.
	if b.Winner() != Empty || !b.Full() {
		t.Errorf("%s: not a draw", b)
	}
}

// ---------------------------------------------------------------------------
// Test 3 — minimax takes an immediate win
// ---------------------------------------------------------------------------

// Board OXXO.X.O. — X to move, and the answer is cell 8.
//
// Preconditions, all verified:
//   - X has exactly one immediate win, the right column 2,5,8, at cell 8.
//   - the empty cells are 4, 6 and 8, so the answer is the highest empty
//     cell and the lowest-index tie-break alone cannot produce it.
//   - a depth-blind search picks cell 6 here, so this board fails a player
//     that scores every win alike.
func TestMinimaxTakesImmediateWin(t *testing.T) {
	b := mustParse(t, "OXXO.X.O.")
	if got := b.Turn(); got != X {
		t.Fatalf("%s: Turn()=%d, want X", b, got)
	}
	if got := winningCells(b, X); len(got) != 1 || got[0] != 8 {
		t.Fatalf("%s: winning cells for X are %v, want [8]", b, got)
	}
	pos, err := b.BestMove(X)
	if err != nil {
		t.Fatalf("%s: BestMove(X): %v", b, err)
	}
	if pos != 8 {
		t.Errorf("%s: BestMove(X)=%d, want 8", b, pos)
	}
}

// ---------------------------------------------------------------------------
// Test 4 — minimax blocks an immediate loss
// ---------------------------------------------------------------------------

// Board ....O.XX. — O to move, and the answer is cell 8.
//
// Preconditions, all verified:
//   - O has no win of its own, so correct play cannot prefer a win over the
//     block and still fail this test.
//   - X has exactly one threat, so exactly one block exists.
//   - the empty cells are 0,1,2,3,5,8 and the block is the highest of them,
//     so the lowest-index tie-break alone cannot produce it.
func TestMinimaxBlocksImmediateLoss(t *testing.T) {
	b := mustParse(t, "....O.XX.")
	if got := b.Turn(); got != O {
		t.Fatalf("%s: Turn()=%d, want O", b, got)
	}
	if got := winningCells(b, O); len(got) != 0 {
		t.Fatalf("%s: O has an immediate win at %v, board is unsound", b, got)
	}
	if got := winningCells(b, X); len(got) != 1 || got[0] != 8 {
		t.Fatalf("%s: X threats are %v, want exactly [8]", b, got)
	}
	pos, err := b.BestMove(O)
	if err != nil {
		t.Fatalf("%s: BestMove(O): %v", b, err)
	}
	if pos != 8 {
		t.Errorf("%s: BestMove(O)=%d, want 8 (the only block)", b, pos)
	}
}

// ---------------------------------------------------------------------------
// Test 5 — perfect play from an empty board always draws
// ---------------------------------------------------------------------------

// This test is thin. A depth-blind broken player draws here too, so it proves
// very little on its own. Test 6 and test 7 are the real oracles.
func TestPerfectPlayFromEmptyBoardDraws(t *testing.T) {
	var b Board
	// Every first move scores 0, so the strict > tie-break returns cell 0.
	if pos, err := b.BestMove(X); err != nil || pos != 0 {
		t.Fatalf("empty board: BestMove(X)=(%d,%v), want (0,nil)", pos, err)
	}
	for b.Winner() == Empty && !b.Full() {
		p := b.Turn()
		pos, err := b.BestMove(p)
		if err != nil {
			t.Fatalf("%s: BestMove(%d): %v", b, p, err)
		}
		if err := b.Move(pos, p); err != nil {
			t.Fatalf("%s: Move(%d,%d): %v", b, pos, p, err)
		}
	}
	if got := b.Winner(); got != Empty {
		t.Errorf("%s: Winner()=%d, want Empty", b, got)
	}
	if !b.Full() {
		t.Errorf("%s: Full()=false, want true", b)
	}
}

// ---------------------------------------------------------------------------
// Test 6 — the perfect player never loses, from either seat
// ---------------------------------------------------------------------------

// This is an outside truth. It knows nothing about the search. It asserts a
// fact about tic-tac-toe that a book can tell you: a perfect player never
// loses, whichever seat it takes.
func TestPerfectPlayerNeverLoses(t *testing.T) {
	for _, hero := range []Player{X, O} {
		calls := 0
		var play func(b Board)
		play = func(b Board) {
			if w := b.Winner(); w != Empty {
				if w != hero {
					t.Fatalf("hero %d lost on board %s", hero, b)
				}
				return
			}
			if b.Full() {
				return
			}
			mover := b.Turn()
			if mover == hero {
				calls++
				pos, err := b.BestMove(hero)
				if err != nil {
					t.Fatalf("%s: BestMove(%d): %v", b, hero, err)
				}
				c := b
				if err := c.Move(pos, hero); err != nil {
					t.Fatalf("%s: Move(%d,%d): %v", b, pos, hero, err)
				}
				play(c)
				return
			}
			for i := 0; i < 9; i++ {
				if b.cells[i] != Empty {
					continue
				}
				c := b
				if err := c.Move(i, mover); err != nil {
					t.Fatalf("%s: Move(%d,%d): %v", b, i, mover, err)
				}
				play(c)
			}
		}
		play(Board{})
		t.Logf("hero %d: %d BestMove calls", hero, calls)
	}
}

// ---------------------------------------------------------------------------
// Test 7 — take the win, in every position that has one
// ---------------------------------------------------------------------------

// This is the test that cannot be fooled. A depth-blind player fails it 392
// times out of 2358.
func TestTakesTheWinEverywhere(t *testing.T) {
	buildTables()
	if got, want := len(winPositions), 2358; got != want {
		t.Errorf("win-available positions=%d, want %d", got, want)
	}
	bad := 0
	for _, b := range winPositions {
		mover := b.Turn()
		wins := winningCells(b, mover)
		pos, err := b.BestMove(mover)
		if err != nil {
			t.Fatalf("%s: BestMove(%d): %v", b, mover, err)
		}
		ok := false
		for _, w := range wins {
			if w == pos {
				ok = true
			}
		}
		if !ok {
			bad++
			if bad <= 10 {
				t.Errorf("%s: %d played %d, but the wins are %v", b, mover, pos, wins)
			}
		}
	}
	if bad > 0 {
		t.Errorf("missed an immediate win in %d of %d positions", bad, len(winPositions))
	}
}

// ---------------------------------------------------------------------------
// Test 8 — never throw away a draw
// ---------------------------------------------------------------------------

// Test 7 only checks won positions. A player that turns a draw into a loss
// slips past every other test in this file.
func TestNeverThrowsAwayADraw(t *testing.T) {
	buildTables()
	if got, want := len(drawPositions), 1052; got != want {
		t.Errorf("drawn positions=%d, want %d", got, want)
	}
	bad := 0
	for _, b := range drawPositions {
		mover := b.Turn()
		pos, err := b.BestMove(mover)
		if err != nil {
			t.Fatalf("%s: BestMove(%d): %v", b, mover, err)
		}
		c := b
		if err := c.Move(pos, mover); err != nil {
			t.Fatalf("%s: Move(%d,%d): %v", b, pos, mover, err)
		}
		if got := outcome(c, mover); got != 0 {
			bad++
			if bad <= 10 {
				t.Errorf("%s: %d played %d and the draw became %d (%s)", b, mover, pos, got, c)
			}
		}
	}
	if bad > 0 {
		t.Errorf("threw away the draw in %d of %d positions", bad, len(drawPositions))
	}
}

// ---------------------------------------------------------------------------
// Test 9 — a win on the ninth mark still counts
// ---------------------------------------------------------------------------

// This is the test for the terminal order. It fails when the search asks
// Full() before Winner(), because the last mark then scores as a draw.
func TestWinOnTheNinthMark(t *testing.T) {
	b := mustParse(t, "XOXO.XXOO")
	if got := b.Turn(); got != X {
		t.Fatalf("%s: Turn()=%d, want X", b, got)
	}
	pos, err := b.BestMove(X)
	if err != nil {
		t.Fatalf("%s: BestMove(X): %v", b, err)
	}
	if pos != 4 {
		t.Fatalf("%s: BestMove(X)=%d, want 4", b, pos)
	}
	if err := b.Move(pos, X); err != nil {
		t.Fatalf("%s: Move(4,X): %v", b, err)
	}
	if got := b.Winner(); got != X {
		t.Errorf("%s: Winner()=%d, want X", b, got)
	}
	if !b.Full() {
		t.Errorf("%s: Full()=false, want true", b)
	}
}

// ---------------------------------------------------------------------------
// Test 10 — the legality rule is exactly right
// ---------------------------------------------------------------------------

// Walk all 3^9 cell arrays and collect what Parse accepts. Walk the game tree
// and collect what a real game reaches. The two sets must match, in both
// directions. The expected size, 5478, comes from outside this code.
func TestLegalityRuleIsExactlyRight(t *testing.T) {
	buildTables()

	accepted := make(map[Board]bool, 6000)
	marks := [3]byte{'.', 'X', 'O'}
	var text [9]byte
	for n := 0; n < 19683; n++ {
		v := n
		for i := 0; i < 9; i++ {
			text[i] = marks[v%3]
			v /= 3
		}
		b, err := Parse(string(text[:]))
		if err == nil {
			accepted[b] = true
			continue
		}
		if !errors.Is(err, ErrIllegalBoard) {
			t.Fatalf("%q: Parse gave %v, want ErrIllegalBoard", text, err)
		}
	}

	if got, want := len(allReachable), 5478; got != want {
		t.Errorf("reachable positions=%d, want %d", got, want)
	}
	if got, want := len(accepted), 5478; got != want {
		t.Errorf("Parse accepts %d positions, want %d", got, want)
	}
	for b := range accepted {
		if !allReachable[b] {
			t.Errorf("%s: Parse accepts it, but no game reaches it", b)
		}
	}
	for b := range allReachable {
		if !accepted[b] {
			t.Errorf("%s: a game reaches it, but Parse rejects it", b)
		}
	}
}

// ---------------------------------------------------------------------------
// Test 11 — Parse and String round-trip
// ---------------------------------------------------------------------------

func TestParseStringRoundTrip(t *testing.T) {
	buildTables()
	for b := range allReachable {
		s := b.String()
		if len(s) != 9 {
			t.Fatalf("%s: String() is %d characters, want 9", s, len(s))
		}
		got, err := Parse(s)
		if err != nil {
			t.Fatalf("%s: Parse of its own String(): %v", s, err)
		}
		if got != b {
			t.Errorf("round-trip changed the board: %s became %s", b, got)
		}
	}
}

// ---------------------------------------------------------------------------
// Test 12 — Move rejects, names the error, and changes nothing
// ---------------------------------------------------------------------------

func TestMoveRejectsNamesTheErrorAndChangesNothing(t *testing.T) {
	// The zero value is the empty board.
	var zero Board
	if zero.Winner() != Empty || zero.Full() || zero.Turn() != X {
		t.Errorf("zero Board is not an empty position: %s", zero)
	}
	if got := zero.String(); got != "........." {
		t.Errorf("zero Board String()=%q, want %q", got, ".........")
	}
	if got := mustParse(t, "........."); got != zero {
		t.Errorf("Parse(\".........\")=%s, want the zero Board", got)
	}

	won := mustParse(t, "XXXOO....")   // game over, O would be to move
	drawn := mustParse(t, "XXOOOXXXO") // full, no winner
	if drawn.Winner() != Empty || !drawn.Full() {
		t.Fatalf("%s: wanted a full board with no winner", drawn)
	}
	oneMark := mustParse(t, "X........")

	cases := []struct {
		name  string
		board Board
		pos   int
		p     Player
		want  error
	}{
		{"empty player", zero, 0, Empty, ErrBadPlayer},
		{"player 7", zero, 0, Player(7), ErrBadPlayer},
		{"bad player beats every other fault", won, -1, Empty, ErrBadPlayer},
		{"position -1", zero, -1, X, ErrOutOfRange},
		{"position 9", zero, 9, X, ErrOutOfRange},
		{"range beats game over", won, 9, X, ErrOutOfRange},
		{"game already won", won, 5, O, ErrGameOver},
		{"board already full", drawn, 0, X, ErrGameOver},
		{"game over beats occupied", won, 0, O, ErrGameOver},
		{"cell taken", oneMark, 0, O, ErrOccupied},
		{"occupied beats turn", oneMark, 0, X, ErrOccupied},
		{"not your turn", zero, 0, O, ErrNotYourTurn},
		{"not your turn again", oneMark, 1, X, ErrNotYourTurn},
	}
	for _, tc := range cases {
		b := tc.board
		before := b
		err := b.Move(tc.pos, tc.p)
		if !errors.Is(err, tc.want) {
			t.Errorf("%s: Move(%d,%d) on %s gave %v, want %v", tc.name, tc.pos, tc.p, before, err, tc.want)
		}
		if b != before {
			t.Errorf("%s: a rejected move changed the board: %s became %s", tc.name, before, b)
		}
	}

	// A good move is accepted and writes exactly one cell.
	b := zero
	if err := b.Move(4, X); err != nil {
		t.Fatalf("Move(4,X) on the empty board: %v", err)
	}
	if got := b.String(); got != "....X...." {
		t.Errorf("after Move(4,X) the board is %s, want ....X....", b)
	}

	// BestMove has its own three named errors, and always returns -1.
	bestCases := []struct {
		name  string
		board Board
		p     Player
		want  error
	}{
		{"bad player", zero, Empty, ErrBadPlayer},
		{"game won", won, O, ErrGameOver},
		{"board full", drawn, X, ErrGameOver},
		{"not your turn", zero, O, ErrNotYourTurn},
	}
	for _, tc := range bestCases {
		pos, err := tc.board.BestMove(tc.p)
		if !errors.Is(err, tc.want) {
			t.Errorf("%s: BestMove(%d) on %s gave %v, want %v", tc.name, tc.p, tc.board, err, tc.want)
		}
		if pos != -1 {
			t.Errorf("%s: BestMove returned %d, want -1", tc.name, pos)
		}
	}

	// Parse names its own two errors.
	parseCases := []struct {
		text string
		want error
	}{
		{"", ErrBadFormat},
		{"XXXXXXXX", ErrBadFormat},
		{"XXXXXXXXXX", ErrBadFormat},
		{"XXX OO. .", ErrBadFormat},
		{"xxxoo....", ErrBadFormat},
		{"XXXXX....", ErrIllegalBoard},
		{"OOOOO....", ErrIllegalBoard},
		{"XXXOOO...", ErrIllegalBoard},
	}
	for _, tc := range parseCases {
		if _, err := Parse(tc.text); !errors.Is(err, tc.want) {
			t.Errorf("Parse(%q) gave %v, want %v", tc.text, err, tc.want)
		}
	}
}

// ---------------------------------------------------------------------------
// Test 13 — the race detector gets something to watch
// ---------------------------------------------------------------------------

// go test -race only watches code that really runs in two goroutines at once.
// Without this test the flag reports clean on a package full of races.
//
// Each goroutine owns its own copy of the board. 64 goroutines is far below
// the race detector's ceiling of about 8128.
func TestConcurrentBestMoveAgrees(t *testing.T) {
	start := mustParse(t, "XOX.O....")
	want, err := start.BestMove(X)
	if err != nil {
		t.Fatalf("%s: BestMove(X): %v", start, err)
	}

	const workers, rounds = 64, 50
	got := make([]int, workers)
	var wg sync.WaitGroup
	for w := 0; w < workers; w++ {
		wg.Add(1)
		go func(w int) {
			defer wg.Done()
			mine := start // an independent copy
			last := -2
			for r := 0; r < rounds; r++ {
				pos, err := mine.BestMove(X)
				if err != nil {
					last = -3
					break
				}
				last = pos
			}
			got[w] = last
		}(w)
	}
	wg.Wait()
	for w, g := range got {
		if g != want {
			t.Errorf("goroutine %d got %d, want %d", w, g, want)
		}
	}
	if start.String() != "XOX.O...." {
		t.Errorf("the shared start board changed to %s", start)
	}
}

// ---------------------------------------------------------------------------
// Live-position count, measured against the outside numbers in the spec.
// ---------------------------------------------------------------------------

func TestLivePositionCount(t *testing.T) {
	buildTables()
	if got, want := len(livePositions), 4520; got != want {
		t.Errorf("live positions=%d, want %d", got, want)
	}
}
