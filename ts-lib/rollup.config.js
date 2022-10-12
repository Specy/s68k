import esbuild from 'rollup-plugin-esbuild'
import typescript from '@rollup/plugin-typescript';
const name = require('./package.json').main.replace(/\.js$/, '')

const bundle = config => ({
    ...config,
    input: 'src/index.ts',
    external: id => !/^[./]/.test(id),
})

export default [
    bundle({
        input: 'src/index.ts',
        plugins: [typescript(),esbuild()],
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
    })
]