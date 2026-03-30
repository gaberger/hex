```markdown
# ADR-2310231200: Build Crypto Portfolio Dashboard with Live Charts

## Status
proposed

## Date
2023-10-23

## Drivers
- User need for a comprehensive and interactive crypto portfolio management tool.
- Demand for real-time data visualization of crypto asset performance.
- Need for maintainable and scalable architecture.

## Context
The project requires the development of a crypto portfolio dashboard that allows users to visualize their investments with live price charts, sparklines for historical trends, and portfolio profit and loss tracking. Utilizing technologies like TypeScript, Vite, and Chart.js offers advantages including type safety, fast build times, and powerful charting capabilities. 

Given the need for a robust, maintainable solution, this dashboard will adhere to hexagonal architecture principles. Thus, we aim to define clear boundaries across domain logic, application use cases, and external integrations, ensuring that the core domain remains decoupled from user interface concerns and other external services.

Existing ADRs in the project relate to different aspects of API construction, but this decision focuses solely on the user-interactive components of the dashboard. The dashboard’s architecture must efficiently support dynamic data updates while providing a responsive user experience.

## Decision
We will build a crypto portfolio dashboard using a hexagonal architecture approach. The implementation will follow a layered strategy, starting with the use cases in the domain layer to handle the core logic of portfolio management and data retrieval. 

The first phase will focus on establishing the domain and use case layers, making sure to define the necessary data models and business logic for tracking portfolio assets and their values. Subsequently, we will develop the primary adapters that will encompass the user interface components built with Vite and Chart.js for displaying real-time data.

The initial phase will map primarily to hex layers 0-2 (domain, use cases) while the second phase will address layers 3-5 (primary adapters, secondary adapters, and composition root).

## Consequences

### Positive
- Provides a clear separation of concerns, allowing independent development of the core logic and UI.
- Facilitates easier testing and maintenance by adhering to hexagonal architecture principles.

### Negative
- Initial development complexity due to adherence to a structured architecture may prolong the first release.
- Dependency on real-time data sources may complicate the implementation and require careful management of API integrations.

### Neutral
- The architecture's modular nature allows for potential future enhancements without significant refactoring.

## Implementation

### Phases
1. **Phase 1** — Create domain models and use cases for tracking crypto assets and their values.
2. **Phase 2** — Develop primary adapters with Vite and Chart.js for delivering live data visualizations.

### Affected Layers
- [x] domain/
- [x] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [x] usecases/
- [x] composition-root

### Migration Notes
None
```