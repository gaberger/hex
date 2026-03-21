export interface DesignToken {
  name: string;
  value: string;
  category: 'color' | 'spacing' | 'typography' | 'shadow' | 'radius';
}

export const tokens: DesignToken[] = [
  { name: 'color-primary', value: '#6366f1', category: 'color' },
  { name: 'color-secondary', value: '#8b5cf6', category: 'color' },
  { name: 'spacing-sm', value: '0.5rem', category: 'spacing' },
  { name: 'spacing-md', value: '1rem', category: 'spacing' },
  { name: 'spacing-lg', value: '2rem', category: 'spacing' },
  { name: 'radius-sm', value: '4px', category: 'radius' },
  { name: 'radius-md', value: '8px', category: 'radius' },
  { name: 'font-size-base', value: '16px', category: 'typography' },
  { name: 'shadow-sm', value: '0 1px 2px rgba(0,0,0,0.05)', category: 'shadow' },
];
