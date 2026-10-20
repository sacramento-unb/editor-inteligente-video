"""Real HTTP/FFmpeg smoke test, using a temporary database and synthetic media.

Run after cargo build: python3 backend/smoke_test.py
"""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def main():
    binary = Path(__file__).parent / 'target/debug/video-editor'
    with tempfile.TemporaryDirectory(prefix='videolab-test-') as directory:
        root = Path(directory)
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            port = listener.getsockname()[1]
        base = f'http://127.0.0.1:{port}'
        env = {**os.environ, 'BIND_ADDR': f'127.0.0.1:{port}',
               'DATABASE_URL': f'sqlite://{root}/db.sqlite', 'STORAGE_ROOT': str(root / 'storage')}
        process = subprocess.Popen([str(binary.resolve())], env=env, stdout=subprocess.DEVNULL)
        try:
            def call(path, method='GET', data=None, headers=None, status=200):
                if isinstance(data, dict):
                    headers = {'Content-Type': 'application/json', **(headers or {})}
                    data = json.dumps(data).encode()
                req = urllib.request.Request(base + path, data=data, headers=headers or {}, method=method)
                try:
                    response = urllib.request.urlopen(req, timeout=15)
                except urllib.error.HTTPError as error:
                    response = error
                with response:
                    payload = response.read()
                    assert response.status == status, (path, response.status, payload[:500])
                    if 'application/json' in response.headers.get('Content-Type', ''):
                        return json.loads(payload)
                    return payload, response.headers

            for attempt in range(100):
                try:
                    call('/health')
                    break
                except urllib.error.URLError:
                    if process.poll() is not None:
                        raise RuntimeError('Backend exited before becoming ready')
                    time.sleep(.1)
            else:
                raise AssertionError('Backend did not become ready')

            def complete(job_id):
                for attempt in range(240):
                    job = call('/api/jobs/' + job_id)
                    assert job['status'] != 'FAILED', job
                    if job['status'] == 'COMPLETED':
                        return job
                    time.sleep(.1)
                raise AssertionError('Job timed out')

            def upload(project_id, path, mime):
                boundary = 'videolab-smoke-boundary'
                body = (f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="{path.name}"\r\nContent-Type: {mime}\r\n\r\n').encode() + path.read_bytes() + f'\r\n--{boundary}--\r\n'.encode()
                result = call(f'/api/projects/{project_id}/media', 'POST', body,
                              {'Content-Type': f'multipart/form-data; boundary={boundary}'}, 201)[0]
                complete(result['job_id'])
                return result['media_id']

            clip = root / 'demo.mp4'
            subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'color=black:s=320x180:r=25',
                            '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000', '-t', '4',
                            '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-c:a', 'aac', str(clip)], check=True)
            project = call('/api/projects', 'POST', {'name': 'Projeto de teste'}, status=201)['id']
            other = call('/api/projects', 'POST', {'name': 'Outro projeto'}, status=201)['id']
            path = '/api/projects/' + project
            call(path, 'PATCH', {'name': 'Vídeo geral'}, status=204)
            assert call(path)['project']['name'] == 'Vídeo geral'
            media = upload(project, clip, 'video/mp4')
            stream = f'{path}/media/{media}/content'
            data, headers = call(stream, headers={'Range': 'bytes=0-99'}, status=206)
            assert len(data) == 100 and headers['Content-Range'].startswith('bytes 0-99/')
            call(f'/api/projects/{other}/media/{media}/content', status=404)
            complete(call(path + '/analyze', 'POST', status=202)['job_id'])
            assert call(path + '/timeline'), 'Analysis must exist when job is COMPLETED'
            cut = {'media_id': media, 'start_ms': 1000, 'end_ms': 2000}
            call(path + '/render', 'POST', {'manual_cuts': [{**cut, 'start_ms': -1}]}, status=400)
            call(path + '/render', 'POST', {'manual_cuts': [{**cut, 'media_id': 'foreign'}]}, status=400)
            call(path + '/render', 'POST', {'manual_cuts': [{**cut, 'start_ms': 0, 'end_ms': 4000}]}, status=400)
            rendered = call(path + '/render', 'POST', {'manual_cuts': [cut], 'normalize_audio': True}, status=202)
            result = json.loads(complete(rendered['job_id'])['result_json'])
            assert 2900 <= result['duration_ms'] <= 3100, result
            download = f'{path}/render-plans/{rendered["render_plan_id"]}/download'
            data, headers = call(download)
            assert len(data) > 1000 and 'attachment' in headers['Content-Disposition']
            assert call(path + '/renders')[0]['status'] == 'COMPLETED'
            second = upload(project, clip, 'video/mp4')
            call(path + '/media/order', 'PATCH', {'media_ids': [second, second]}, status=400)
            call(path + '/media/order', 'PATCH', {'media_ids': [second, media]}, status=204)
            assert [item['file']['id'] for item in call(path + '/media')] == [second, media]
            call(path + '/render', 'POST', {'manual_cuts': [cut]}, status=400)
            concatenated = call(path + '/render', 'POST', {}, status=202)
            result = json.loads(complete(concatenated['job_id'])['result_json'])
            assert 7900 <= result['duration_ms'] <= 8300, result
            wav = root / 'audio.wav'
            subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'sine=sample_rate=48000',
                            '-t', '16', '-ac', '2', str(wav)], check=True)
            assert wav.stat().st_size > 2 * 1024 * 1024
            upload(other, wav, 'audio/wav')
            call(f'/api/projects/{other}/render', 'POST', {}, status=400)
            print('OK: rename, >2MB upload, byte ranges, isolation, analysis, cuts, normalization, downloads, order and concat.')
        finally:
            process.terminate()
            process.wait(timeout=10)


if __name__ == '__main__':
    main()
