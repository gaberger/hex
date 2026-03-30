package main

import (
	"net/http"
	"github.com/gin-gonic/gin"
	"sync"
)

type URLMapping struct {
	ShortURL string `json:"short_url"`
	Original string `json:"original_url"`
}

var urlStore = struct{
	sync.RWMutex
	store map[string]string
}{store: make(map[string]string)}

func main() {
	r := gin.Default()
	
	r.POST("/shorten", func(c *gin.Context) {
		var mapping URLMapping
		if err := c.ShouldBindJSON(&mapping); err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid input"})
			return
		}
		
		urlStore.Lock()
		urlStore.store[mapping.ShortURL] = mapping.Original
		urlStore.Unlock()

		c.JSON(http.StatusCreated, mapping)
	})

	r.GET("/:short_url", func(c *gin.Context) {
		shortURL := c.Param("short_url")
		
		urlStore.RLock()
		originalURL, exists := urlStore.store[shortURL]
		urlStore.RUnlock()

		if !exists {
			c.JSON(http.StatusNotFound, gin.H{"error": "URL not found"})
			return
		}

		c.Redirect(http.StatusMovedPermanently, originalURL)
	})

	r.Run(":3000")
}