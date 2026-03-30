package main

import(
	"log"
	"net/http"
	"time"

	"github.com/gin-gonic/gin"
)

type URL struct {
	ID        uint32     `json:"id"`
	Original  string     `json:"original"`
	CreatedAt time.Time  `json:"created_at"`
}

var (
	urls = []URL{}
	nextID uint32 = 1
)

func main() {
	r := gin.Default()

	r.POST("/urls", func(c *gin.Context) {
		var input struct {
			Original string `json:"original"`
		}
		if err := c.ShouldBindJSON(&input); err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
			return
		}

		url := URL{
			ID:        nextID,
			Original:  input.Original,
			CreatedAt: time.Now(),
		}
		urls = append(urls, url)
		nextID++

		c.JSON(http.StatusCreated, url)
	})

	r.GET("/urls", func(c *gin.Context) {
		c.JSON(http.StatusOK, urls)
	})

	r.DELETE("/urls/:id", func(c *gin.Context) {
		id := c.Param("id")
		var found bool
		for i, url := range urls {
			if url.ID == uint32(id) {
				urls = append(urls[:i], urls[i+1:]...)
				found = true
				break
			}
		}
		if found {
			c.Status(http.StatusNoContent)
		} else {
			c.Status(http.StatusNotFound)
		}
	})

	log.Fatal(r.Run(":8080"))
}