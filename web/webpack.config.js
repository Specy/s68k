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
  mode: "development",
  devServer: {
    port: 3000,
    static: path.resolve(__dirname)
  }
};
