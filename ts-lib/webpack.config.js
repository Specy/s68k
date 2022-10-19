import { resolve, join } from "path";

export default{
  mode: 'development',
  entry: './src/index.ts',
  output: {
    filename: 'index.js',
    module: true,
    path: resolve("./", 'dist')
  },
  experiments: {
    asyncWebAssembly: true,
  },
  module: {
    rules: [
      {
        test: /\.tsx?$/,
        use: 'ts-loader',
        exclude: /node_modules/,
      },
    ],
  },
  resolve: {
    extensions: ['.tsx', '.ts', '.js'],
  }
}