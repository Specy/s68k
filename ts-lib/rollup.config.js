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
          dest: 'dist/web',
        }, {
          src: '../README.md',
          dest: ['dist/web', './'],
        }],
        flatten: false
      }),
      copy({
        targets: [{
          src: 'src/**/*.wasm',
          dest: "dist/web",
        },]
      }),
      typescript(),
      esbuild(),
      wasm(),
    ],
    input: "src/index.ts",
    output: [
      {
        format: 'es',
        dir: "dist/web",
        sourcemap: true,
      },
    ],
  }
]
