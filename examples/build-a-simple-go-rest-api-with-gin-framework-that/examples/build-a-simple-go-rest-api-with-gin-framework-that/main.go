package main

import (
	"net/http"

	"github.com/gin-gonic/gin"
)

func main() {
	r := gin.Default()

	r.GET("/health", func(c *gin.Context) {
		c.JSON(http.StatusOK, gin.H{"status": "ok", "service": "Plan: build a simple Go REST API with gin framework that has CRUD endpoints for todos"})
	})

	r.Run(":8080")
}
