// Cells are numbered 0..8, left to right, then top to bottom.
// Cell 0 is top-left. Cell 4 is the centre. Cell 8 is bottom-right.
//
//	0 | 1 | 2
//	3 | 4 | 5
//	6 | 7 | 8
//
// Package gamettt plays tic-tac-toe. The board is a value. The player is
// perfect.
package gamettt

import "errors"

// Player is a mark on the board, or the absence of one.
type Player uint8

const (
	// Empty is an unplayed cell. Winner returns it to mean "no winner".
	Empty Player = 0
	// X is the player who moves first.
	X Player = 1
	// O is the player who moves second.
	O Player = 2
)

// Board is a tic-tac-toe position.
//
// The cell array is unexported on purpose. A caller outside this package
// cannot build a board that no real game could reach, so every board that
// leaves this package has passed a rule check.
//
// A Board copies completely: it holds no slice and no pointer. Assignment
// gives an independent second board, and == compares positions.
//
// The zero value is the empty board, which is a legal starting position.
type Board struct{ cells [9]Player }

// lines are the eight ways to win, written once.
var lines = [8][3]int{
	{0, 1, 2}, {3, 4, 5}, {6, 7, 8},
	{0, 3, 6}, {1, 4, 7}, {2, 5, 8},
	{0, 4, 8}, {2, 4, 6},
}

// The error values. A caller must be able to tell a bad request from a
// finished game, so every failure has a name. Compare with errors.Is.
var (
	ErrBadPlayer    = errors.New("gamettt: player must be X or O")
	ErrOutOfRange   = errors.New("gamettt: position must be 0..8")
	ErrGameOver     = errors.New("gamettt: game is already over")
	ErrOccupied     = errors.New("gamettt: cell is already taken")
	ErrNotYourTurn  = errors.New("gamettt: it is not that player's turn")
	ErrIllegalBoard = errors.New("gamettt: board cannot occur in a real game")
	ErrBadFormat    = errors.New("gamettt: board text must be 9 characters of X, O or .")
)

// counts returns the number of X marks and the number of O marks.
func (b Board) counts() (nx, no int) {
	for _, c := range b.cells {
		switch c {
		case X:
			nx++
		case O:
			no++
		}
	}
	return nx, no
}

// Move places player's mark on cell pos.
//
// The five checks run in a fixed order, and the order is part of the
// contract: bad player, out of range, game over, occupied, not your turn.
// No check writes anything, so when Move returns an error the board is
// exactly as it was.
//
// Concurrency: one *Board belongs to one goroutine. There is no mutex and
// there is no atomic, because a move is read, think, write. Two goroutines
// that share one *Board will lose a move, and no lock inside this type can
// prevent that without breaking copying and ==.
func (b *Board) Move(pos int, player Player) error {
	if player != X && player != O {
		return ErrBadPlayer
	}
	if pos < 0 || pos > 8 {
		return ErrOutOfRange
	}
	if b.Winner() != Empty || b.Full() {
		return ErrGameOver
	}
	if b.cells[pos] != Empty {
		return ErrOccupied
	}
	if b.Turn() != player {
		return ErrNotYourTurn
	}
	b.cells[pos] = player
	return nil
}

// Winner returns X or O when that player holds a line, and Empty otherwise.
//
// Empty means "no winner". It does not mean "draw". To find a draw, ask for
// Winner() == Empty and Full() == true together.
func (b Board) Winner() Player {
	for _, l := range lines {
		p := b.cells[l[0]]
		if p != Empty && p == b.cells[l[1]] && p == b.cells[l[2]] {
			return p
		}
	}
	return Empty
}

// Full reports whether every cell holds a mark.
func (b Board) Full() bool {
	for _, c := range b.cells {
		if c == Empty {
			return false
		}
	}
	return true
}

// Turn returns the player to move: X when the mark counts are equal, and O
// otherwise. It never returns Empty, so check Winner and Full first if you
// want to know whether the game still runs.
func (b Board) Turn() Player {
	nx, no := b.counts()
	if nx == no {
		return X
	}
	return O
}

// String returns the position as exactly 9 characters of X, O and '.', with
// no spaces. It is the format Parse reads.
func (b Board) String() string {
	out := make([]byte, 9)
	for i, c := range b.cells {
		switch c {
		case X:
			out[i] = 'X'
		case O:
			out[i] = 'O'
		default:
			out[i] = '.'
		}
	}
	return string(out)
}

// holds reports whether p holds at least one line.
func (b Board) holds(p Player) bool {
	for _, l := range lines {
		if b.cells[l[0]] == p && b.cells[l[1]] == p && b.cells[l[2]] == p {
			return true
		}
	}
	return false
}

// legal reports whether this position can occur in a real game.
//
//  1. The X count minus the O count is 0 or 1.
//  2. X and O do not both hold a line. Rules 3 and 4 already reject that,
//     because they demand two different mark counts at the same time, so
//     this rule stays a comment and not a branch.
//  3. If X holds a line, the X count is the O count plus 1.
//  4. If O holds a line, the X count equals the O count.
//
// Checked against all 19683 cell arrays: the rule accepts exactly the 5478
// positions a real game can reach.
func (b Board) legal() bool {
	nx, no := b.counts()
	if d := nx - no; d != 0 && d != 1 {
		return false
	}
	if b.holds(X) && nx != no+1 {
		return false
	}
	if b.holds(O) && nx != no {
		return false
	}
	return true
}

// Parse reads a position from exactly 9 characters of X, O and '.'.
//
// It rejects any other text with ErrBadFormat, and any position a real game
// cannot reach with ErrIllegalBoard.
func Parse(s string) (Board, error) {
	var b Board
	if len(s) != 9 {
		return Board{}, ErrBadFormat
	}
	for i := 0; i < 9; i++ {
		switch s[i] {
		case 'X':
			b.cells[i] = X
		case 'O':
			b.cells[i] = O
		case '.':
			b.cells[i] = Empty
		default:
			return Board{}, ErrBadFormat
		}
	}
	if !b.legal() {
		return Board{}, ErrIllegalBoard
	}
	return b, nil
}
