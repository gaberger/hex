# Food Delivery Service (TypeScript)

A TypeScript implementation of a food delivery service demonstrating **hexagonal architecture** with hex autonomous execution.

## Purpose

This example validates hex's **TypeScript support** and demonstrates:
- Language-agnostic hexagonal architecture enforcement
- Autonomous workplan execution on TypeScript projects
- BuildAdapter detection via `package.json`/`tsconfig.json`
- TypeScript-specific compile gates (`npx tsc --noEmit`)

## Architecture

```
src/
├── core/
│   ├── domain/          # Pure business logic (Order, OrderStatus)
│   ├── ports/           # Interface contracts (IOrderRepository)
│   └── usecases/        # Application logic
└── adapters/
    ├── primary/         # HTTP, CLI entry points
    └── secondary/       # Database, external service implementations
```

### Hexagonal Rules (Enforced)

- **Domain** imports nothing (zero external dependencies)
- **Ports** import domain only (for value types)
- **Usecases** import domain + ports only
- **Adapters** import ports + domain only (never other adapters)

Validated by: `hex analyze .`

## Domain Model

### Order Entity
```typescript
class Order {
  id: OrderId;
  customerId: CustomerId;
  restaurantId: RestaurantId;
  items: OrderItem[];
  status: OrderStatus;
  totalAmount: Money;
  createdAt: Date;
  updatedAt: Date;
}
```

### OrderStatus (State Machine)
```
Pending → Confirmed → Preparing → OutForDelivery → Delivered
                  ↓
               Cancelled
```

## Setup

```bash
# Install dependencies
npm install

# Type check
npm run typecheck

# Run tests
npm test

# Build
npm run build
```

## Autonomous Workplan Execution

This example includes a test workplan that builds the Order domain autonomously:

```bash
# Execute workplan (requires hex with TypeScript support)
hex plan execute workplan-order-domain.json
```

**Expected outcome** (once TypeScript integration is complete):
- 4 phases, 6 tasks execute autonomously
- Domain entities created in `src/core/domain/`
- Port interfaces created in `src/core/ports/`
- Adapter implementations in `src/adapters/secondary/`
- Tests generated with vitest

## Test Workplan

See `workplan-order-domain.json` for the complete execution plan.

**Phases**:
1. Domain: Order entity + OrderStatus enum
2. Port: IOrderRepository interface
3. Adapter: InMemoryOrderRepository implementation
4. Test: Domain and adapter test suites

**Evidence gates** (TypeScript-specific):
```bash
npx tsc --noEmit           # Compile check
grep -q 'export' file.ts   # Exports present
test -f src/path/file.ts   # File exists
```

## Current Status

**BuildAdapter**: ✅ Detects TypeScript correctly  
**Workplan Execution**: ⚠️ In progress (workplan_executor.rs hardcoded to `cargo check`)

See: `docs/TEST-TYPESCRIPT-SUPPORT.md` for integration status.

## Success Criteria

- [ ] `hex analyze .` detects TypeScript project
- [ ] `hex plan execute workplan-order-domain.json` completes without errors
- [ ] Compile gate uses `npx tsc --noEmit` (not `cargo check`)
- [ ] All 6 tasks complete autonomously
- [ ] Generated code passes `hex analyze .` boundary checks
- [ ] Tests run with vitest
- [ ] Architecture grade ≥ B

## Related

- **Rust example**: `examples/task-board/` (production-ready)
- **Test guide**: `docs/TEST-TYPESCRIPT-SUPPORT.md`
- **Workplan**: `workplan-order-domain.json`
- **ADR**: ADR-018 (BuildAdapter)
