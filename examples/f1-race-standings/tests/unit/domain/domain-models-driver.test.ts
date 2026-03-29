import{ Driver } from '../../../src/core/domain/domain-models-driver.js';

describe('Driver', () => {
  describe('Happy Path', () => {
    it('should create a valid driver with all properties', () => {
      const driver = new Driver('1', 'John Doe', 'Team A');
      expect(driver.id).toBe('1');
      expect(driver.name).toBe('John Doe');
      expect(driver.team).toBe('Team A');
    });
  });

  describe('Error Cases', () => {
    it('should throw for invalid id type', () => {
      expect(() => new Driver(1, 'John Doe', 'Team A')).toThrow();
    });

    it('should throw for invalid name type', () => {
      expect(() => new Driver('1', 123, 'Team A')).toThrow();
    });

    it('should throw for invalid team type', () => {
      expect(() => new Driver('1', 'John Doe', 456)).toThrow();
    });
  });

  describe('Edge Cases', () => {
    it('should handle empty id', () => {
      expect(() => new Driver('', 'John Doe', 'Team A')).toThrow();
    });

    it('should handle empty name', () => {
      expect(() => new Driver('1', '', 'Team A')).toThrow();
    });

    it('should handle empty team', () => {
      expect(() => new Driver('1', 'John Doe', '')).toThrow();
    });
  });
});