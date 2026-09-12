package gamettt

// The sign convention, pinned once:
//
//	A score is always from the view of the player who is about to move.
//	Plus is good for that player. Minus is bad for that player.
//	Each step deeper negates the score it gets back.
//
// The depth convention, pinned once:
//
//	depth counts the moves played since this search started. The root is
//	depth 0.
//
// Scores: a win is 10-depth, a loss is depth-10, a draw is 0. The depth term
// is not polish. Without it a win now and a win in five moves both score the
// same, and the player wanders instead of taking the win. A depth-blind
// search misses an immediate win in 392 of the 2358 positions that have one.
//
// There is no transposition cache, no alpha-beta and no goroutine fan-out.
// The whole tree from the empty board is 549946 nodes, which is small. A
// cache is the one real correctness landmine here, because a score measured
// from one starting board must never be reused for a different one.

// BestMove returns the best cell for player, found by a full negamax search.
//
// The receiver is a value, so the search can never touch the caller's board.
//
// On failure it returns -1 and never 0, because a 0 looks like a real move
// and a caller who drops the error will play it:
//
//   - player is not X or O: ErrBadPlayer
//   - the game is won or the board is full: ErrGameOver
//   - it is not that player's turn: ErrNotYourTurn
//
// Ties go to the lowest cell number, because the comparison is a strict >.
// With >= the tie-break would silently become the highest cell number. From
// the empty board every move scores 0, so BestMove returns 0. A later change
// such as "prefer the centre" must therefore change a test as well.
func (b Board) BestMove(player Player) (int, error) {
	if player != X && player != O {
		return -1, ErrBadPlayer
	}
	if b.Winner() != Empty || b.Full() {
		return -1, ErrGameOver
	}
	if b.Turn() != player {
		return -1, ErrNotYourTurn
	}
	best, bestScore := -1, -100
	for pos := 0; pos < 9; pos++ {
		if b.cells[pos] != Empty {
			continue
		}
		c := b
		c.cells[pos] = player
		score := -nega(c, 1)
		if score > bestScore { // strict >: the lowest cell number wins a tie
			bestScore, best = score, pos
		}
	}
	return best, nil
}

// nega returns the score of b from the view of the player to move in b.
//
// The terminal order matters. Winner() is asked first, then Full(). A board
// that is both won and full is a win, not a draw. Ask Full() first and every
// win made with the ninth mark disappears from the tree.
func nega(b Board, depth int) int {
	if b.Winner() != Empty {
		return depth - 10 // the player to move has already lost
	}
	if b.Full() {
		return 0
	}
	mover := b.Turn()
	best := -100
	for pos := 0; pos < 9; pos++ {
		if b.cells[pos] != Empty {
			continue
		}
		c := b
		c.cells[pos] = mover
		if score := -nega(c, depth+1); score > best {
			best = score
		}
	}
	return best
}
