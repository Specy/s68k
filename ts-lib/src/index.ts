import { add } from './a/test';
import { S68k as _S68k, SemanticError as _SemanticError } from './pkg/s68k';

type CompilationResult =
    | {
          errors: SemanticError[];
      }
    | {
          interpreter: S68k;
      };

class SemanticError {
    error: _SemanticError;
    constructor(error: _SemanticError) {
        this.error = error;
    }
    getMessage() {
        return this.error.wasm_get_message();
    }
    getLineIndex() {
        const result = add(1, 2);
        return this.error.wasm_get_line();
    }
}

export class S68k {
    private _s68k: _S68k;
    constructor(code: string) {
        this._s68k = new _S68k(code);
    }

    static compile(code: string): CompilationResult {
        const s68k = new S68k(code);
        const errors = s68k.semanticCheck();
        if (errors.length > 0) return { errors };
        return { interpreter: s68k };
    }
    public semanticCheck(): SemanticError[] {
        const errorWrapper = this._s68k.wasm_semantic_check();
        const errors: SemanticError[] = [];
        for (let i = 0; i < errorWrapper.get_length(); i++) {
            errors.push(new SemanticError(errorWrapper.get_error_at_index(i)));
        }
        return errors;
    }
}

