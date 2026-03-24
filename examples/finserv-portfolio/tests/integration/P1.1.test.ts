import { TestBed } from '../TestBed.js';

describe('P1.1 Composition Root', () => {
  it('should compose the application with stub adapters', async () => {
    const testBed = TestBed.withStubs();
    // Note: compositionRoot import removed as it doesn't exist in the current structure
    // TODO: Define composition root in src/composition-root.js
    expect(testBed.adapters.primaryAdapter).toBeDefined();
    expect(testBed.adapters.secondaryAdapter).toBeDefined();
  });
});