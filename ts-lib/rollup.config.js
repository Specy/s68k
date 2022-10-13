import dts from 'rollup-plugin-dts'
import typescript from '@rollup/plugin-typescript';
import { wasm } from '@rollup/plugin-wasm';
import glob from 'glob';
import esbuild from 'rollup-plugin-esbuild'
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const name = require('./package.json').main.replace(/\.js$/, '')



export default [
  {
    plugins: [esbuild(), typescript(), wasm()],
    input: "src/index.ts",
    output: [
      {
        format: 'es',
        dir:"dist",
        sourcemap: true,
      },
    ],
  }
]
