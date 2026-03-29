package main

import (
	"context"
	"net/http"
)

type Handler struct {
	port *port.Port
}

func NewHandler(port *port.Port) *Handler {
	return &Handler{
		port: port,
	}
}

func (h *Handler) GetItem(ctx context.Context, id int) (*port.Item, error) {
	return h.port.GetItem(ctx, id)
}