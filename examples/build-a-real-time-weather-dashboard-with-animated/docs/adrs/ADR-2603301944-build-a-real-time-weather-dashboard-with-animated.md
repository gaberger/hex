```markdown
# ADR-2310281230: Build Real-Time Weather Dashboard with Geolocation

## Status
proposed

## Date
2023-10-28

## Drivers
- User need for an interactive weather dashboard to visualize real-time data.
- Demand for a modern, responsive user interface with animated visuals.

## Context
The goal is to create a real-time weather dashboard that showcases current weather conditions, a 5-day forecast, and utilizes geolocation capabilities to provide users with relevant data based on their location. This application will leverage TypeScript and Vite for its development, ensuring type safety and rapid build times. User experience is critical, as we aim to provide an engaging interface through animated SVG icons that represent weather conditions dynamically.

The hexagonal architecture will help us separate concerns effectively. The domain will manage business logic mainly focused on weather data and forecasts. The ports will define interfaces for how external data sources (e.g., weather APIs) can be accessed. Adapters will handle the interaction with these APIs and present the data to the use cases responsible for the application’s workflow. This modular approach will facilitate easier maintenance and scalability.

## Decision
We will implement a real-time weather dashboard by utilizing a domain-driven design approach where the core business logic resides within the domain layer. We will build out ports to create interfaces for fetching weather data and injecting geolocation capabilities. The external data fetching, including integration with weather APIs, will be handled in the secondary adapters, while the primary adapters will focus on the presentation layer, rendering animated SVG icons and the user interface.

The implementation will be structured in two phases: first, we will establish the core domain logic for weather data handling and geolocation. Second, we will develop the primary adapters to build out the user interface, leveraging the data provided by the use cases.

## Consequences

### Positive
- The use of TypeScript will enhance code reliability and reduce runtime errors.
- Animated SVG icons will improve user engagement and experience.

### Negative
- Higher initial complexity in defining domain models and interfaces.
- Possible performance impacts if the real-time data fetching is not optimized.

### Neutral
- The project scope might expand to include forecasting features not initially planned.

## Implementation

### Phases
1. Phase 1 — Establish domain models and interfaces for weather data and geolocation (Hex Layer 1-2).
2. Phase 2 — Develop user interface with animated SVG icons utilizing the data provided by the domain and ports (Hex Layer 4).

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [ ] adapters/secondary/
- [x] usecases/
- [ ] composition-root

### Migration Notes
None
```