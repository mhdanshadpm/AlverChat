import { spawn } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
const count = Math.max(1, Number(process.argv[2] ?? 2));
const base = resolve('.dev-peers');
mkdirSync(base, { recursive:true });
const children = Array.from({length:count},(_,i)=>spawn('pnpm',['exec','vite','--host','127.0.0.1','--port',String(1420+i),'--strictPort'],{stdio:'inherit',env:{...process.env,ALVERCHAT_MOCK_PEER:String(i+1),ALVERCHAT_DATA_DIR:resolve(base,`peer-${i+1}`)}}));
process.on('SIGINT',()=>{for(const child of children)child.kill('SIGINT');process.exit(0)});
