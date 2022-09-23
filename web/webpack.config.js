const path = require('path');
module.exports = {
  entry: "./bootstrap.js",
  experiments: {
    asyncWebAssembly: true,
  },
  output: {
    path: path.resolve(__dirname, "dist"),
    filename: "bootstrap.js",
  },
  mode: "development",
  devServer: {
    port: 3000,
    static: path.resolve(__dirname)
  }
};
