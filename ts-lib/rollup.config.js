import dts from 'rollup-plugin-dts'
import typescript from '@rollup/plugin-typescript';
import { wasm } from '@rollup/plugin-wasm';

import esbuild from 'rollup-plugin-esbuild'

const name = require('./package.json').main.replace(/\.js$/, '')

const bundle = config => ({
  ...config,
  input: 'src/index.ts',
  external: id => !/^[./]/.test(id),
})

export default [
  bundle({
    plugins: [esbuild(), typescript(), wasm()],
    output: [
      {
        file: `${name}.js`,
        format: 'cjs',
        sourcemap: true,
      },
      {
        file: `${name}.mjs`,
        format: 'es',
        sourcemap: true,
      },
    ],
  }),
  bundle({
    input: '/pkg/s68k.d.ts',
    output: [{ file: 'dist/s68k.d.ts', format: 'es' }],
    plugins: [dts()],
  }),
  bundle({
    plugins: [dts()],
    output: {
      file: `${name}.d.ts`,
      format: 'es',
    },
  }),
]
