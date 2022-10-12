'use strict';

Object.defineProperty(exports, "__esModule", { value: true });
exports.S68k = void 0;
const test_1 = require("./a/test");
const s68k_1 = require("./pkg/s68k");
class SemanticError {
  error;
  constructor(error) {
    this.error = error;
  }
  getMessage() {
    return this.error.wasm_get_message();
  }
  getLineIndex() {
    const result = (0, test_1.add)(1, 2);
    console.log(result);
    return this.error.wasm_get_line();
  }
}
class S68k {
  _s68k;
  constructor(code) {
    this._s68k = new s68k_1.S68k(code);
  }
  static compile(code) {
    const s68k = new S68k(code);
    const errors = s68k.semanticCheck();
    if (errors.length > 0)
      return { errors };
    return { interpreter: s68k };
  }
  semanticCheck() {
    const errorWrapper = this._s68k.wasm_semantic_check();
    const errors = [];
    for (let i = 0; i < errorWrapper.get_length(); i++) {
      errors.push(new SemanticError(errorWrapper.get_error_at_index(i)));
    }
    return errors;
  }
}
exports.S68k = S68k;
//# sourceMappingURL=index.js.map
