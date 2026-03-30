package main

import (
	"net/http"

	"github.com/gin-gonic/gin"
)

type URLPair struct {
	ID  string `json:"id"`
	URL string `json:"url"`
}

var urlStorage = make(map[string]string)

func main() {
	router := gin.Default()

	router.POST("/shorten", shortenURL)
	router.GET("/:id", redirectURL)

	router.Run(":8080")
}

func shortenURL(c *gin.Context) {
	var newURL URLPair
	if err := c.ShouldBindJSON(&newURL); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	urlID := generateID()
	urlStorage[urlID] = newURL.URL
	newURL.ID = urlID

	c.JSON(http.StatusCreated, newURL)
}

func redirectURL(c *gin.Context) {
	id := c.Param("id")
	if url, exists := urlStorage[id]; exists {
		c.Redirect(http.StatusMovedPermanently, url)
		return
	}
	c.JSON(http.StatusNotFound, gin.H{"error": "URL not found"})
}

func generateID() string {
	// In a real application, this would be a more sophisticated ID generation strategy
	return "short.ly/" + string(len(urlStorage)+1)
}