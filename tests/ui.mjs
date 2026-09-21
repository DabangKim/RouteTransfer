import { chromium } from '@playwright/test';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
const root=await mkdtemp('/private/tmp/rt-ui-');
await mkdir(root+'/files');await writeFile(root+'/files/photo.jpg','fixture');
const backend=spawn('src-tauri/target/debug/rt-harness',[root+'/db']);
const lines=createInterface({input:backend.stdout});let pending;let queue=Promise.resolve();
lines.on('line',s=>{pending?.(JSON.parse(s));pending=null});
function call(action,args={}){const result=queue.then(()=>new Promise((resolve,reject)=>{pending=r=>r.error?reject(new Error(r.error)):resolve(r.ok);backend.stdin.write(JSON.stringify({action,args})+'\n')}));queue=result.catch(()=>{});return result}
await call('save_settings',{connect_timeout:30,auth_timeout:30,io_timeout:120,local_path:root+'/files'});
const browser=await chromium.launch({executablePath:process.env.CHROME_PATH||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true});
try{
const page=await browser.newPage({viewport:{width:1440,height:960}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
await page.exposeFunction('testInvoke',(cmd,args)=>{assert.equal(cmd,'dispatch');return call(args.action,args.args)});
await page.addInitScript(()=>{window.isTauri=true;window.__TAURI_INTERNALS__={invoke:(cmd,args)=>window.testInvoke(cmd,args)}});
await page.goto('http://127.0.0.1:1420');await page.getByLabel('프로필 이름',{exact:true}).fill('UI 검증 경로');await page.getByLabel('호스트',{exact:true}).fill('127.0.0.1');await page.getByLabel('사용자',{exact:true}).fill('tester');await page.getByRole('button',{name:'프로필 저장',exact:true}).click();
await page.getByRole('button',{name:'UI 검증 경로 서버 1개'}).waitFor();
await page.getByRole('button',{name:'＋ 서버 추가'}).click();await page.getByLabel('호스트',{exact:true}).fill('ubuntu.internal');await page.getByLabel('사용자',{exact:true}).fill('tester');await page.getByRole('button',{name:'프로필 저장',exact:true}).click();
await page.getByRole('button',{name:'UI 검증 경로 서버 2개'}).waitFor();assert.equal((await call('snapshot')).profiles[0].hops.length,2);
await page.screenshot({path:'tests/results/connections.png',fullPage:true});
await page.getByRole('button',{name:'파일 전송',exact:true}).click();await page.getByRole('button',{name:'photo.jpg',exact:true}).waitFor();await page.getByLabel('photo.jpg 선택').check();assert.equal(await page.getByRole('button',{name:'업로드 →'}).isEnabled(),false);
await page.screenshot({path:'tests/results/transfer.png',fullPage:true});
await page.getByRole('button',{name:'작업 기록',exact:true}).click();await page.getByRole('button',{name:'감사 기록',exact:true}).click();await page.getByText('프로필 저장',{exact:true}).first().waitFor();
await page.getByRole('button',{name:'설정',exact:true}).click();await page.screenshot({path:'tests/results/settings.png',fullPage:true});
await page.setViewportSize({width:1120,height:760});assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>window.innerWidth),false);
assert.deepEqual(errors,[]);await writeFile('tests/results/ui.json',JSON.stringify({realRustBackend:true,profileCreateAndEdit:true,localBrowse:true,selection:true,disconnectedTransferDisabled:true,audit:true,screens:['connections','transfer','settings'],minimumWindowNoHorizontalOverflow:true,pageErrors:errors},null,2)+'\n');console.log('UI smoke checks passed using real product Core');
}finally{await browser.close();backend.stdin.end();lines.close();await rm(root,{recursive:true,force:true})}
