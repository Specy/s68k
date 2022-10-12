import { SemanticError as SemanticError$1 } from './pkg/s68k.js';

declare type CompilationResult = {
    errors: SemanticError[];
} | {
    interpreter: S68k;
};
declare class SemanticError {
    error: SemanticError$1;
    constructor(error: SemanticError$1);
    getMessage(): string;
    getLineIndex(): any;
}
declare class S68k {
    private _s68k;
    constructor(code: string);
    static compile(code: string): CompilationResult;
    semanticCheck(): SemanticError[];
}

export { S68k };
