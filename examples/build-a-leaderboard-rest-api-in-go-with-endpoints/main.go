package main

import (
	"net/http"
	"sort"
	"sync"

	"github.com/gin-gonic/gin"
)

type Score struct {
	Player string `json:"player"`
	Points int    `json:"points"`
}

var (
	scores      = make([]Score, 0)
	scoresMutex sync.Mutex
)

func main() {
	r := gin.Default()

	r.POST("/submit", submitScore)
	r.GET("/leaderboard", getLeaderboard)

	r.Run(":8080")
}

func submitScore(c *gin.Context) {
	var score Score
	if err := c.ShouldBindJSON(&score); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid input"})
		return
	}

	scoresMutex.Lock()
	scores = append(scores, score)
	scoresMutex.Unlock()

	c.JSON(http.StatusCreated, score)
}

func getLeaderboard(c *gin.Context) {
	scoresMutex.Lock()
	defer scoresMutex.Unlock()

	// Sort scores in descending order
	sort.Slice(scores, func(i, j int) bool {
		return scores[i].Points > scores[j].Points
	})

	c.JSON(http.StatusOK, scores)
}