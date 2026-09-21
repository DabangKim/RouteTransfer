"""Real Ubuntu/OpenSSH integration experiment in isolated Docker containers."""
import asyncio
import json
import os
import platform
import secrets
import subprocess
import tempfile
import time
from contextlib import asynccontextmanager
from datetime import datetime, timezone
from pathlib import Path
import asyncssh
from verify_local import CHUNK, route, snapshot, upload

IMAGE = 'routetransfer-poc-ubuntu:local'
RESULT = Path(__file__).parent / 'results' / 'ubuntu.json'


def docker(*args, **kwargs):
    return subprocess.check_output(['docker', *args], text=True, **kwargs).strip()


@asynccontextmanager
async def lab(base, count):
    identifier = 'rt-poc-' + secrets.token_hex(5)
    containers = []
    nodes = []
    docker('network', 'create', identifier)
    try:
        for index in range(count):
            root = base / f'server-{index + 1}'
            root.mkdir(parents=True)
            root.chmod(0o777)  # Synthetic test data only, inside TemporaryDirectory.
            name = f'{identifier}-{index + 1}'
            password = secrets.token_urlsafe(24)
            args = ['run', '-d', '--name', name, '--network', identifier,
                    '--env', 'TEST_PASSWORD', '--mount',
                    f'type=bind,source={root},target=/data']
            if index == 0:
                args += ['-p', '127.0.0.1::22']
            args.append(IMAGE)
            docker(*args, env={**os.environ, 'TEST_PASSWORD': password})
            containers.append(name)
            for _ in range(100):
                ready = subprocess.run(['docker', 'exec', name, 'test', '-f', '/run/sshd.pid'],
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                if ready.returncode == 0:
                    break
                await asyncio.sleep(0.1)
            else:
                raise RuntimeError('Test SSH server did not become ready')
            public = docker('exec', name, 'cat', '/etc/ssh/ssh_host_ed25519_key.pub')
            port = int(docker('port', name, '22/tcp').rsplit(':', 1)[1]) if index == 0 else 22
            nodes.append(dict(host='127.0.0.1' if index == 0 else name, port=port,
                              password=password, root=root,
                              fingerprint=asyncssh.import_public_key(public).get_fingerprint()))
        yield nodes, containers
    finally:
        for name in reversed(containers):
            subprocess.run(['docker', 'rm', '-f', name], check=False, stdout=subprocess.DEVNULL)
        subprocess.run(['docker', 'network', 'rm', identifier], check=False, stdout=subprocess.DEVNULL)


async def test_route(base, count):
    source = base / 'photos'
    (source / '2026' / '한글 폴더').mkdir(parents=True)
    (source / 'empty').mkdir()
    for index in range(24):
        (source / '2026' / '한글 폴더' / f'photo {index:03}.bin').write_bytes(
            bytes([index]) * (CHUNK + index * 31))
    async with lab(base / 'lab', count) as (nodes, containers):
        version = docker('exec', containers[-1], 'dpkg-query', '-W', 'openssh-server')
        async with route(nodes) as connections:
            async with connections[-1].start_sftp_client() as sftp:
                for folder in sorted(p for p in source.rglob('*') if p.is_dir()):
                    await sftp.makedirs('/data/photos/' + folder.relative_to(source).as_posix(), exist_ok=True)
                for file in sorted(p for p in source.rglob('*') if p.is_file()):
                    await upload(sftp, file, '/data/photos/' + file.relative_to(source).as_posix())
                downloaded = base / 'download'
                downloaded.mkdir()
                await sftp.get('/data/photos', str(downloaded), recurse=True)
        assert snapshot(source) == snapshot(downloaded / 'photos')
        assert (downloaded / 'photos' / 'empty').is_dir()
        assert all(not list(n['root'].iterdir()) for n in nodes[:-1])
        # Verify rejection using the real Ubuntu authentication and host key paths.
        for options, expected in [({'bad_password': count-1}, asyncssh.PermissionDenied),
                                  ({'bad_key': count-1}, asyncssh.HostKeyNotVerifiable)]:
            try:
                async with route(nodes, **options):
                    raise AssertionError('Bad credentials/key were accepted')
            except expected:
                pass
        # Inject a disconnect while replacing a separate file.
        complete = source / 'complete.bin'
        pending = source / 'pending.bin'
        complete.write_bytes(b'C' * CHUNK)
        pending.write_bytes(b'D' * CHUNK * 8)
        final_root = nodes[-1]['root']
        (final_root / 'pending.bin').write_bytes(b'old target')
        completed = set()
        attempts = {complete.name: 0, pending.name: 0}
        async with route(nodes) as connections:
            async with connections[-1].start_sftp_client() as sftp:
                attempts[complete.name] += 1
                await upload(sftp, complete, '/data/' + complete.name)
                completed.add(complete.name)
                old_mtime = (final_root / complete.name).stat().st_mtime_ns
                attempts[pending.name] += 1
                try:
                    await upload(sftp, pending, '/data/' + pending.name,
                                 interrupt=connections[0].abort)
                except (ConnectionError, asyncssh.Error, OSError):
                    pass
                else:
                    raise AssertionError('Expected disconnect')
        assert (final_root / 'pending.bin').read_bytes() == b'old target'
        assert 0 < (final_root / 'pending.bin.rt-poc-part').stat().st_size < pending.stat().st_size
        async with route(nodes) as connections:
            async with connections[-1].start_sftp_client() as sftp:
                for file in (complete, pending):
                    if file.name in completed:
                        continue
                    attempts[file.name] += 1
                    assert await upload(sftp, file, '/data/' + file.name) == file.stat().st_size
                    completed.add(file.name)
        assert (final_root / complete.name).stat().st_mtime_ns == old_mtime
        assert (final_root / pending.name).read_bytes() == pending.read_bytes()
        assert not (final_root / 'pending.bin.rt-poc-part').exists()
        assert attempts == {'complete.bin': 1, 'pending.bin': 2}
        return dict(servers=count, openssh=version, roundtrip_files=24,
                    roundtrip_bytes=1581420, sha256_match=True,
                    folder_structure=True, intermediate_data_files=0,
                    bad_password_rejected=True, wrong_host_key_rejected=True,
                    disconnect_retry_attempts=attempts, retry_from_zero=True,
                    previous_target_preserved=True)


async def main():
    report = dict(timestamp=datetime.now(timezone.utc).isoformat(), platform=platform.platform(),
                  image=IMAGE, image_id=docker('image', 'inspect', '--format', '{{.Id}}', IMAGE),
                  scope='Ubuntu 24.04/OpenSSH containers, AsyncSSH client, synthetic small fixtures',
                  tests=[])
    with tempfile.TemporaryDirectory(prefix='rt-ubuntu-') as tmp:
        for count in (2, 4):
            path = Path(tmp) / str(count)
            path.mkdir()
            start = time.monotonic()
            try:
                details = await asyncio.wait_for(test_route(path, count), 180)
                result = dict(name=f'ubuntu_{count}_servers', status='pass', details=details)
            except Exception as exc:
                result = dict(name=f'ubuntu_{count}_servers', status='fail',
                              error=f'{type(exc).__name__}: {exc}')
            result['seconds'] = round(time.monotonic() - start, 3)
            report['tests'].append(result)
            print(json.dumps(result, ensure_ascii=False), flush=True)
    report['passed'] = all(t['status']=='pass' for t in report['tests'])
    RESULT.write_text(json.dumps(report, indent=2, ensure_ascii=False)+'\n')
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    asyncio.run(main())
