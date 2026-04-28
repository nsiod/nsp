import antfu from '@antfu/eslint-config';

export default antfu({
  type: 'app',
  typescript: true,
  react: true,
  stylistic: {
    indent: 2,
    quotes: 'single',
    semi: true,
  },
  ignores: [
    'dist',
    'node_modules',
    'src/app/routeTree.gen.ts',
  ],
  rules: {
    'no-alert': 'off',
    'no-console': ['warn', { allow: ['warn', 'error'] }],
    'react/prefer-namespace-import': 'off',
    'react-refresh/only-export-components': 'off',
    'antfu/top-level-function': 'off',
    'ts/no-use-before-define': 'off',
    'style/arrow-parens': ['error', 'always'],
    'unused-imports/no-unused-vars': [
      'error',
      {
        args: 'after-used',
        argsIgnorePattern: '^_',
        varsIgnorePattern: '^_',
      },
    ],
  },
});
