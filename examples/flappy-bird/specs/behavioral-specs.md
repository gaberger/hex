# Flappy Bird — Behavioral Specifications

These specs define WHAT the game does, not HOW it's implemented.
They serve as acceptance criteria for the validation judge.

## Physics
- BS-1: Gravity pulls the bird downward continuously when playing
- BS-2: When the player taps, the bird moves UPWARD (negative Y in screen coords)
- BS-3: The bird's visual rotation matches its movement direction (nose up when rising, nose down when falling)
- BS-4: The bird dies when hitting the GROUND (bottom of screen), but NOT the ceiling
- BS-5: The bird dies when colliding with any part of a pipe

## Game Flow
- BS-6: Game starts in 'ready' state showing "TAP TO START"
- BS-7: First tap transitions to 'playing' AND makes the bird flap upward
- BS-8: During gameplay, each tap makes the bird flap upward
- BS-9: On collision, game transitions to 'gameover' showing score and "TAP TO RESTART"
- BS-10: Tapping during gameover resets to 'ready' state

## Scoring
- BS-11: Score increments by 1 each time the bird fully passes a pipe
- BS-12: High score persists between game sessions
- BS-13: High score updates only when current score exceeds it

## Sign Convention Contract
- Y-axis: positive = downward (screen coordinates)
- Gravity: positive value (e.g., 980) -> adds to velocity -> bird falls
- Flap strength: NEGATIVE value (e.g., -280) -> sets velocity negative -> bird rises
- Velocity: positive = falling, negative = rising
