# Hex Web Design System

A hexagonal architecture pattern applied to web page design.

## Structure

```
src/
  core/
    domain/         # Pure content types (HeroSection, CardItem, DesignToken)
    ports/          # Component & data interfaces
  adapters/
    primary/        # UI rendering (HTML generation)
    secondary/      # Data fetching (CMS, CDN)
  pages/
    composition-root.ts  # Wires adapters → domain
```

## Hex Principles Applied

| Layer | Web Design |
|-------|------------|
| Domain | Content types, design tokens — zero UI deps |
| Ports | Component interfaces (CardPort, HeroPort) |
| Primary Adapters | HTML generation (card-adapter.ts) |
| Secondary Adapters | Data fetching (CmsAdapter, CdnAdapter) |
| Composition Root | Wires everything together |

## Run

```bash
cd examples/web-design-system
npm install
npm run dev
```
