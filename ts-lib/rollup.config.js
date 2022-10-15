import typescript from '@rollup/plugin-typescript';
import { wasm } from '@rollup/plugin-wasm';
import copy from 'rollup-plugin-copy'
import esbuild from 'rollup-plugin-esbuild'


export default [
  {
    plugins: [
      copy({
        targets: [{
          src: 'src/**/*.d.ts',
          dest: 'dist/',
        }, {
          src: '../README.md',
          dest: ['dist/', './'],
        }],
        flatten: false
      }),
      typescript(),
      esbuild(),
      wasm(),
    ],
    input: "src/index.ts",
    output: [
      {
        format: 'es',
        dir: "dist",
        sourcemap: true,
      },
    ],
  }
]
