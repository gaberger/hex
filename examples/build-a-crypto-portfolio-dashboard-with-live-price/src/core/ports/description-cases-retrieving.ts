package ports

import "context"

// PortfolioAsset represents a crypto asset in the portfolio
type PortfolioAsset struct {
	ID     string  `json:"id"`
	Name   string  `json:"name"`
	Value  float64 `json:"value"`
}

// PortfolioRepository is the interface that defines the methods to manage portfolio assets.
type PortfolioRepository interface {
	GetAllAssets(ctx context.Context) ([]PortfolioAsset, error)
	AddAsset(ctx context.Context, asset PortfolioAsset) error
	UpdateAsset(ctx context.Context, asset PortfolioAsset) error
	DeleteAsset(ctx context.Context, id string) error
}