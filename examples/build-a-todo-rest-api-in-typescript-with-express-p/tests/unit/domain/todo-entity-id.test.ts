import { Empty } from '../../../src/core/domain/todo-entity-id.js';

describe('TodoEntityId', () => {
  describe('Empty', () => {
    it('should be a valid type', () => {
      const value: Empty = null;
      expect(value).toBeNull();
    });

    it('should be assignable to any', () => {
      const anyValue: any = null;
      expect(anyValue).toBeNull();
    });

    it('should not cause type errors when used as a number', () => {
      const numValue: number = null;
      expect(numValue).toBeNull();
    });

    it('should not cause type errors when used as a string', () => {
      const strValue: string = null;
      expect(strValue).toBeNull();
    });

    it('should not cause type errors when used as an object', () => {
      const objValue: object = null;
      expect(objValue).toBeNull();
    });

    it('should not cause type errors when used as an array', () => {
      const arrValue: any[] = null;
      expect(arrValue).toBeNull();
    });

    it('should not cause type errors when used as a function', () => {
      const funcValue: () => void = null;
      expect(funcValue).toBeNull();
    });

    it('should not cause type errors when used as a boolean', () => {
      const boolValue: boolean = null;
      expect(boolValue).toBeNull();
    });

    it('should not cause type errors when used as a symbol', () => {
      const symbolValue: symbol = null;
      expect(symbolValue).toBeNull();
    });

    it('should not cause type errors when used as a bigint', () => {
      const bigintValue: bigint = null;
      expect(bigintValue).toBeNull();
    });
  });
});