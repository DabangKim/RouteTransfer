"""Exercise the same Core used by Tauri against isolated Ubuntu SSH servers."""
import asyncio, hashlib, json, sys, tempfile, uuid, sqlite3
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'poc'))
from verify_ubuntu import lab
ROOT = Path(__file__).resolve().parents[1]
class Client:
    async def start(self, directory):
        self.p = await asyncio.create_subprocess_exec(str(ROOT/'src-tauri/target/debug/rt-harness'), str(directory), stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE)
        return self
    async def call(self, action, **args):
        self.p.stdin.write((json.dumps(dict(action=action,args=args))+'\n').encode()); await self.p.stdin.drain()
        result=json.loads(await asyncio.wait_for(self.p.stdout.readline(),30))
        if 'error' in result: raise RuntimeError(result['error'])
        return result['ok']
    async def idle(self, nodes=None):
        for _ in range(3000):
            s=await self.call('snapshot'); r=s['runtime']; c=r['challenge']
            if c and nodes:
                if c['kind']=='host':
                    assert c['fingerprint'] == nodes[r['hop']]['fingerprint']
                    await self.call('answer',id=c['id'],accepted=True)
                else: await self.call('answer',id=c['id'],accepted=True,password=nodes[r['hop']]['password'],save=False)
            if not s['busy']: return s
            await asyncio.sleep(.02)
        raise TimeoutError('operation timed out')
    async def plan(self, direction, source_dir, name, destination):
        listing=await self.call('list',path=str(source_dir),remote=direction=='download')
        page=await self.call('page',**listing)
        paths=[e['path'] for e in page['entries'] if e['name']==name]
        assert paths, (name,page)
        b=await self.call('prepare',direction=direction,paths=paths,destination=str(destination),**listing)
        s=await self.idle(); j=next(j for j in s['jobs'] if j['batch']['id']==b['id'])
        assert j['batch']['state']=='awaiting_confirmation',s['runtime']['error']
        return b['id']
    async def transfer(self,b,policy='skip',retry=False):
        await self.call('retry' if retry else 'confirm',id=b,policy=policy,ack_issues=True)
        s=await self.idle()
        items=await self.call('items',id=b)
        assert all(i['state'] in ('succeeded','skipped') for i in items), [(i['source'],i['state'],i['error']) for i in items]
        return items
    async def stop(self):
        self.p.stdin.close(); await self.p.wait()
def digest(p): return hashlib.sha256(p.read_bytes()).hexdigest()
async def run(base, count):
    source=base/'photos'; (source/'한글 폴더').mkdir(parents=True); (source/'empty').mkdir()
    for n in range(8): (source/'한글 폴더'/f'photo {n}.jpg').write_bytes(bytes([n])* (65536+n))
    download=base/'download'; download.mkdir()
    async with lab(base/'lab',count) as (nodes,containers):
        c=await Client().start(base/'db')
        try:
            profile=await c.call('save_profile',id=str(uuid.uuid4()),name=f'{count} servers',description='',remote_path='/data',revision=0,hops=[dict(id=str(uuid.uuid4()),alias=f'Server {n}',host=v['host'],port=v['port'],username='poc') for n,v in enumerate(nodes)])
            await c.call('connect',id=profile['id']); s=await c.idle(nodes)
            assert s['runtime']['state']=='ready',s['runtime']['error']
            b=await c.plan('upload',base,'photos','/data'); await c.transfer(b)
            target=nodes[-1]['root']/'photos'
            assert (target/'empty').is_dir()
            for p in source.rglob('*.jpg'): assert digest(p)==digest(target/p.relative_to(source))
            b=await c.plan('download','/data','photos',download); await c.transfer(b)
            for p in source.rglob('*.jpg'): assert digest(p)==digest(download/'photos'/p.relative_to(source))
            b=await c.plan('upload',base,'photos','/data'); items=await c.transfer(b)
            assert sum(i['state']=='skipped' for i in items)==8
            p=source/'한글 폴더'/'photo 0.jpg';p.write_bytes(b'replacement')
            b=await c.plan('upload',base,'photos','/data'); await c.transfer(b,'overwrite')
            assert (target/'한글 폴더'/'photo 0.jpg').read_bytes()==b'replacement'
            b=await c.plan('upload',source/'한글 폴더','photo 0.jpg','/data')
            p.write_bytes(b'changed after planning')
            await c.call('confirm',id=b,policy='skip'); await c.idle()
            items=await c.call('items',id=b); assert items[0]['state']=='needs_review'
            # Kill during a real transfer, then restart with no automatic connection.
            (base/'retry-set').mkdir(); (base/'retry-set'/'a-complete.jpg').write_bytes(b'done'); big=base/'retry-set'/'z-large.bin'; big.write_bytes(b'x'*(32*1024*1024))
            b=await c.plan('upload',base,'retry-set','/data'); await c.call('confirm',id=b,policy='skip')
            for _ in range(1000):
                s=await c.call('snapshot')
                if int(s['runtime']['bytes'] or '0')>65536: break
                await asyncio.sleep(.001)
            else: raise AssertionError('did not observe streaming')
            c.p.kill();await c.p.wait();await c.start(base/'db')
            s=await c.call('snapshot');assert s['runtime']['state']=='disconnected' and not s['busy']
            assert any(i['state']=='interrupted' for i in await c.call('items',id=b))
            completed=nodes[-1]['root']/'retry-set'/'a-complete.jpg'; previous_mtime=completed.stat().st_mtime_ns
            await c.call('connect',id=profile['id']);s=await c.idle(nodes);assert s['runtime']['state']=='ready',s
            items=await c.transfer(b,retry=True);large=next(i for i in items if i['source'].endswith('z-large.bin'));assert large['attempt']==2
            assert completed.stat().st_mtime_ns==previous_mtime
            assert next(i for i in items if i['source'].endswith('a-complete.jpg'))['attempt']==1
            assert digest(big)==digest(nodes[-1]['root']/'retry-set'/'z-large.bin')
            assert len(await c.call('attempts',id=large['id']))==2
            assert not list(nodes[-1]['root'].rglob('.routetransfer-*'))
            assert all(not list(n['root'].iterdir()) for n in nodes[:-1])
            await c.call('disconnect')
            await c.call('connect',id=profile['id'])
            wrong=[dict(n,password='intentionally-incorrect') for n in nodes]
            s=await c.idle(wrong);assert s['runtime']['state']=='failed' and 'AUTH_REJECTED' in s['runtime']['error']
            with sqlite3.connect(base/'db'/'routetransfer.sqlite3') as db:
                db.execute("UPDATE trusted SET data=?",(json.dumps('SHA256:changed-test-key'),))
            await c.call('connect',id=profile['id']);s=await c.idle()
            assert s['runtime']['state']=='failed' and 'HOST_KEY_CHANGED' in s['runtime']['error']
            for f in (base/'db').glob('routetransfer.sqlite3*'):
                if f.is_file():
                    data=f.read_bytes()
                    assert all(n['password'].encode() not in data for n in nodes)
            return dict(hops=count,bad_password_rejected=True,changed_host_key_blocked=True,completed_not_retried=True,password_not_in_database=True,roundtrip_sha256=True,empty_directories=True,skip=True,overwrite=True,source_change_blocked=True,process_restart_recovery=True,retry_from_zero=True,attempts=2,no_intermediate_files=True)
        finally: await c.stop()
async def main():
    results=[]
    with tempfile.TemporaryDirectory(prefix='rt-product-',dir='/private/tmp') as tmp:
        for count in (1,2,4):
            result=await asyncio.wait_for(run(Path(tmp)/str(count),count),240)
            results.append(result);print(json.dumps(result),flush=True)
    (ROOT/'tests/results/product-integration.json').write_text(json.dumps(dict(scope='Real product Core; macOS client and isolated Ubuntu 24.04/OpenSSH; synthetic fixtures up to 32 MiB. No Windows, company network or 100 GB benchmark.',tests=results),indent=2)+'\n')
asyncio.run(main())
