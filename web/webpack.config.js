const path = require('path');
const CopyPlugin = require('copy-webpack-plugin');
module.exports = {
  entry: "./bootstrap.js",
  experiments: {
    asyncWebAssembly: true,
  },
  plugins: [
    new CopyPlugin({
      patterns: [
        { from: 'index.html', to: 'index.html', toType: 'file'},
      ]
    })
  ],
  output: {
    path: path.resolve(__dirname, "build"),
    filename: "bootstrap.js",
  },
  module: {
    rules: [
      {
        test: /\.tsx?$/,
        use: 'ts-loader',
        exclude: /node_modules/,
      },{
				test: /\.css$/,
				use: ['style-loader', 'css-loader']
			},
			{
				test: /\.ttf$/,
				use: ['file-loader']
			}
    ],
  },

  mode: "development",
  devServer: {
    port: 3000,
    static: path.resolve(__dirname)
  }
};
