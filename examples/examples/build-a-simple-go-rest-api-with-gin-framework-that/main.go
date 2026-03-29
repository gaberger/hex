package main

import(
	"github.com/gin-gonic/gin"
	"sync"
)

type Item struct {
	ID    int    `json:"id"`
	Name  string `json:"name"`
}

var (
	todos = []Item{}
	mutex sync.Mutex
)

func main() {
	r := gin.Default()
	r.POST("/todos", createItem)
	r.GET("/todos", listItems)
	r.PUT("/todos/:id", updateItem)
	r.DELETE("/todos/:id", deleteItem)
	r.Run(":3000")
}

func createItem(c *gin.Context) {
	var item Item
	if err := c.ShouldBindJSON(&item); err != nil {
		c.JSON(400, gin.H{"error": err.Error()})
		return
	}
	mutex.Lock()
	defer mutex.Unlock()
	item.ID = len(todos) + 1
	todos = append(todos, item)
	c.JSON(201, item)
}

func listItems(c *gin.Context) {
	mutex.Lock()
	defer mutex.Unlock()
	c.JSON(200, todos)
}

func updateItem(c *gin.Context) {
	var item Item
	if err := c.ShouldBindJSON(&item); err != nil {
		c.JSON(400, gin.H{"error": err.Error()})
		return
	}
	id := c.Param("id")
	mutex.Lock()
	defer mutex.Unlock()
	for i, t := range todos {
		if t.ID == id {
			todos[i] = item
			c.JSON(200, item)
			return
		}
	}
	c.JSON(404, gin.H{"error": "item not found"})
}

func deleteItem(c *gin.Context) {
	id := c.Param("id")
	mutex.Lock()
	defer mutex.Unlock()
	for i, t := range todos {
		if t.ID == id {
			todos = append(todos[:i], todos[i+1:]...)
			c.Status(204)
			return
		}
	}
	c.JSON(404, gin.H{"error": "item not found"})
}