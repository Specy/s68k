import {execSync} from 'child_process';
import fs from "fs/promises"



async function init(){
    execSync('tsc', {stdio: 'inherit'});
    await fs.cp("./src/pkg", "./dist/web/pkg", { recursive: true });
}

init()