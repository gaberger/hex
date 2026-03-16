/**
 * ESLint flat config for hex-intf
 *
 * Intervention B: Enforces domain error-type rules at lint time.
 * - Bans `new Error(` in src/core/domain/ — forces use of DomainError or DomainEvent
 * - Bans `new Error(` in src/core/usecases/ — forces domain-specific error types
 */
import tseslint from '@typescript-eslint/eslint-plugin';
import tsparser from '@typescript-eslint/parser';

export default [
  {
    files: ['src/**/*.ts', 'tests/**/*.ts'],
    languageOptions: {
      parser: tsparser,
      parserOptions: {
        ecmaVersion: 'latest',
        sourceType: 'module',
      },
    },
    plugins: {
      '@typescript-eslint': tseslint,
    },
    rules: {
      '@typescript-eslint/no-unused-vars': ['warn', { argsIgnorePattern: '^_' }],
      '@typescript-eslint/no-explicit-any': 'warn',
    },
  },
  // Intervention B: Ban raw Error() in domain and usecases layers
  {
    files: ['src/core/domain/**/*.ts'],
    rules: {
      'no-restricted-syntax': [
        'error',
        {
          selector: 'NewExpression[callee.name="Error"]',
          message:
            'Domain layer must not throw raw Error(). Use a DomainError subclass or emit a DomainEvent instead. See: src/core/domain/errors.ts',
        },
        {
          selector: 'ThrowStatement > NewExpression[callee.name="TypeError"]',
          message:
            'Domain layer must not throw TypeError. Use a typed DomainError subclass.',
        },
        {
          selector: 'ThrowStatement > NewExpression[callee.name="RangeError"]',
          message:
            'Domain layer must not throw RangeError. Use a typed DomainError subclass.',
        },
      ],
    },
  },
  {
    files: ['src/core/usecases/**/*.ts'],
    rules: {
      'no-restricted-syntax': [
        'error',
        {
          selector: 'NewExpression[callee.name="Error"]',
          message:
            'Usecases must not throw raw Error(). Use a domain-specific error type from core/domain/errors.ts.',
        },
      ],
    },
  },
];
