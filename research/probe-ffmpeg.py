"""Offline FFmpeg pipe feasibility probe; no AirPlay interoperability claim."""
import json
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target' / 'ffmpeg-probe'
OUT.mkdir(parents=True, exist_ok=True)
FFMPEG = shutil.which('ffmpeg')
FFPROBE = shutil.which('ffprobe')
if not FFMPEG or not FFPROBE:
    raise SystemExit('ffmpeg and ffprobe must be on PATH')


def run(args, data=None):
    result = subprocess.run(args, input=data, capture_output=True, check=False, timeout=90)
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors='replace'))
    return result


def ff(*args, data=None):
    return run([FFMPEG, '-hide_banner', '-loglevel', 'error', '-y', *map(str, args)], data)


def probe(path):
    result = run([FFPROBE, '-v', 'error', '-select_streams', 'v:0',
                  '-count_frames', '-show_streams', '-show_packets',
                  '-show_entries', 'stream=codec_name,width,height,nb_read_frames:packet=pts_time,dts_time',
                  '-of', 'json', str(path)])
    return json.loads(result.stdout)


results = {'version': run([FFMPEG, '-version']).stdout.decode().splitlines()[0], 'cases': []}
for codec, encoder, extension in [('h264', 'libx264', 'h264'), ('hevc', 'libx265', 'hevc')]:
    source = OUT / ('synthetic.' + extension)
    extra = ['-x265-params', 'log-level=error:pools=1:frame-threads=1'] if codec == 'hevc' else []
    ff('-f', 'lavfi', '-i', 'testsrc2=size=160x90:rate=30:duration=1',
       '-an', '-c:v', encoder, '-preset', 'ultrafast', '-tune', 'zerolatency',
       '-bf', '0', *extra, '-f', codec, source)
    # subprocess pipes are binary, unlike Windows PowerShell text pipelines.
    ff('-f', codec, '-i', 'pipe:0', '-f', 'null', '-', data=source.read_bytes())
    output = OUT / (codec + '-copy.mp4')
    ff('-fflags', '+genpts', '-r', '30', '-f', codec, '-i', 'pipe:0',
       '-c:v', 'copy', output, data=source.read_bytes())
    metadata = probe(output)
    stream = metadata['streams'][0]
    assert stream['codec_name'] == codec and int(stream['nb_read_frames']) == 30
    results['cases'].append({'case': codec + '_binary_pipe_decode_and_stream_copy',
                             'result': 'passed', 'stream': stream,
                             'timing': 'synthetic constant 30 fps, not AirPlay timestamps'})

source = OUT / 'timestamp-gap.ts'
output = OUT / 'timestamp-gap.mkv'
ff('-f', 'lavfi', '-i', 'testsrc2=size=160x90:rate=30:duration=1',
   '-vf', r'setpts=if(lt(N\,15)\,N/(30*TB)\,(N+15)/(30*TB))',
   '-fps_mode', 'passthrough', '-c:v', 'libx264', '-preset', 'ultrafast',
   '-tune', 'zerolatency', '-bf', '0', '-f', 'mpegts', source)
ff('-copyts', '-i', 'pipe:0', '-c:v', 'copy', output, data=source.read_bytes())
a, b = probe(source), probe(output)
pts_a = [float(p['pts_time']) for p in a['packets']]
pts_b = [float(p['pts_time']) for p in b['packets']]
assert len(pts_a) == len(pts_b) == 30
error = max(abs((x-pts_a[0]) - (y-pts_b[0])) for x, y in zip(pts_a, pts_b))
assert error <= 0.0011
assert pts_b[15] - pts_b[14] > 0.5
results['cases'].append({'case': 'timestamped_mpegts_pipe_to_matroska', 'result': 'passed',
                        'frames': len(pts_b), 'maximum_relative_pts_error_seconds': error,
                        'preserved_gap_seconds': pts_b[15] - pts_b[14]})
(OUT / 'results.json').write_text(json.dumps(results, indent=2), encoding='utf-8')
print(json.dumps(results, indent=2))
