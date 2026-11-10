"""Windows live-radio measurement using the real TUI and WASAPI loopback.

Use a CPython with numpy/scipy and SoundCard/pywinpty on PYTHONPATH.
Instrumentation is applied to an ignored copy of the TUI; production sources
and the normal user's host/settings are not changed. Output is under build/latency.
Terminal submission is measured, not physical monitor scanout or speaker acoustics.
"""
import argparse
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / 'build/latency'


def instrument():
    if 'struct Decoder {' not in (ROOT / 'crates/thmr-cli/src/tui/spectrum.rs').read_text(encoding='utf-8'):
        archived = BASE / 'before-replacement/latency_probe.exe'
        if not archived.exists():
            raise RuntimeError('Legacy analyzer has moved to the host; use tool/measure_host_audio.py for current binaries.')
        target = ROOT / 'target/debug/examples/latency_probe.exe'
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(archived, target)
        print('Reusing archived pre-replacement measurement binary; current host measurement: tool/measure_host_audio.py', flush=True)
        return
    dest = BASE / 'instrumented'
    dest.mkdir(parents=True, exist_ok=True)
    shutil.copytree(ROOT / 'crates/thmr-cli/src/tui', dest / 'tui', dirs_exist_ok=True)
    source = (ROOT / 'crates/thmr-cli/src/tui.rs').read_text(encoding='utf-8')
    source = source.replace('mod battlefield;', '#[path="tui/battlefield.rs"] mod battlefield;')
    for name in ['choreography', 'danmaku', 'model', 'render', 'spectrum', 'spectrum_view']:
        source = source.replace(f'mod {name};', f'#[path="tui/{name}.rs"] mod {name};')
    source = source.replace('let mut dirty = true;', 'let mut dirty = true;\n        let mut trace_end: u64;')
    source = source.replace('ui.tick(&state, connection.is_none(), &audio);',
                            'ui.tick(&state, connection.is_none(), &audio);\n                trace_end = audio.trace_end;')
    source = source.replace('last_frame = Instant::now();',
        'crate::trace::draw(trace_end, ui.channel_rms[0]);\n                last_frame = Instant::now();')
    (dest / 'tui.rs').write_text(source, encoding='utf-8')
    path = dest / 'tui/spectrum.rs'
    source = path.read_text(encoding='utf-8')
    source = source.replace('pub struct Spectrum {', 'pub struct Spectrum {\n    pub trace_end: u64,')
    source = source.replace('display: DisplaySpectrum::default(),',
                            'trace_end: 0,\n            display: DisplaySpectrum::default(),')
    source = source.replace('if analyzer.add(samples, rate, channels).is_some() {',
        'let trace_end = crate::trace::pcm(samples, rate, channels);\n                    if analyzer.add(samples, rate, channels).is_some() {')
    source = source.replace('display: analyzer.display.frame,', 'trace_end: trace_end.saturating_sub(analyzer.hop as u64),\n                            display: analyzer.display.frame,')
    path.write_text(source, encoding='utf-8')
    (dest / 'instrumentation.json').write_text(json.dumps({'sample_position':'fft_window_end'}), encoding='utf-8')
    example = ROOT / 'crates/thmr-cli/examples/latency_probe.rs'
    example.parent.mkdir(parents=True, exist_ok=True)
    example.write_text('''// Generated measurement binary, using a copy of the current TUI.
#[path="../../../build/latency/instrumented/tui.rs"] mod tui;
mod trace {
    use std::{fs::File, io::{BufWriter, Write}, sync::{Mutex, OnceLock}, time::{SystemTime, UNIX_EPOCH}};
    struct Trace { pcm: BufWriter<File>, draws: BufWriter<File>, meta: BufWriter<File>, end: u64 }
    static TRACE: OnceLock<Mutex<Trace>> = OnceLock::new();
    fn state() -> &'static Mutex<Trace> { TRACE.get_or_init(|| {
        let dir = std::path::PathBuf::from(std::env::var_os("THMR_TRACE_DIR").unwrap());
        Mutex::new(Trace { pcm: BufWriter::new(File::create(dir.join("source.f32")).unwrap()),
            draws: BufWriter::new(File::create(dir.join("draw.csv")).unwrap()),
            meta: BufWriter::new(File::create(dir.join("decode.csv")).unwrap()), end: 0 })
    }) }
    fn now() -> f64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64() }
    pub fn pcm(samples: &[f32], rate: u32, channels: usize) -> u64 {
        let mut t = state().lock().unwrap();
        for frame in samples.chunks_exact(channels) { t.pcm.write_all(&frame[0].to_le_bytes()).unwrap(); }
        t.end += (samples.len()/channels) as u64;
        let end = t.end;
        writeln!(t.meta, "{},{},{}", now(), end, rate).unwrap();
        end
    }
    pub fn draw(end: u64, level: f32) { let mut t = state().lock().unwrap();
        writeln!(t.draws, "{},{},{}", now(), end, level).unwrap(); }
    pub fn flush() { if let Some(t) = TRACE.get() { let mut t=t.lock().unwrap();
        t.pcm.flush().unwrap(); t.draws.flush().unwrap(); t.meta.flush().unwrap(); } }
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let result = tui::run(thmr_core::client::Client::discover().await?).await;
    trace::flush(); result
}
''', encoding='utf-8')
    try:
        subprocess.run(['cargo', 'build', '-p', 'thmr-cli', '--example', 'latency_probe'], cwd=ROOT, check=True)
    finally:
        example.unlink(missing_ok=True)


def capture(args):
    import numpy as np
    import soundcard as sc
    from winpty import PtyProcess
    directory = BASE / args.name
    directory.mkdir(parents=True, exist_ok=True)
    config = directory / 'config'
    config.mkdir(exist_ok=True)
    (config / 'settings.toml').write_text(f'source = "{args.source}"\nvolume = 45.0\n', encoding='utf-8')
    env = dict(os.environ, THMR_RUNTIME_DIR=str(directory / 'runtime'),
               THMR_CONFIG_DIR=str(config), THMR_TRACE_DIR=str(directory), THMR_NO_SYSTEM_MEDIA='1')
    env.pop('THMR_AUDIO_OUTPUT', None)
    env.pop('THMR_TUI_REDUCED_MOTION', None)
    host_log = (directory / 'host.log').open('wb')
    archived_host = BASE / 'before-replacement/thmrd.exe'
    host_executable = archived_host if archived_host.exists() else ROOT / 'target/debug/thmrd.exe'
    host = subprocess.Popen([str(host_executable)], env=env, stdout=host_log, stderr=host_log)
    endpoint = directory / 'runtime/instance.json'
    import urllib.request
    def call(path, command=None):
        instance = json.loads(endpoint.read_text())
        data = None if command is None else json.dumps({'protocolVersion': 1, 'command': {'command': command}}).encode()
        request = urllib.request.Request(f'http://127.0.0.1:{instance["port"]}{path}', data=data,
            headers={'Authorization': 'Bearer '+instance['token'], 'X-THMR-Protocol': '1', 'Content-Type': 'application/json'})
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=5) as response:
            return json.load(response)
    tui = None
    try:
        for _ in range(100):
            if host.poll() is not None:
                raise RuntimeError(f'host exited: {host.returncode}; inspect host.log')
            if endpoint.exists():
                break
            time.sleep(.1)
        call('/v1/command', 'play')
        for _ in range(90):
            state = call('/v1/state')
            if state['phase'] == 'online':
                break
            time.sleep(1)
        else:
            raise RuntimeError('real stream did not become online: '+str({k:state.get(k) for k in ['phase','error','metadataError']}))
        # Save selected safe metadata only; instance.json and credentials are never reported.
        summary = {k:state.get(k) for k in ['phase', 'source', 'profileId', 'volume']}
        summary['device'] = sc.default_speaker().name
        summary['capture_rate'] = 48000
        manifest = BASE / 'instrumented/instrumentation.json'
        summary['sample_position'] = json.loads(manifest.read_text())['sample_position'] if manifest.exists() else 'decoded_mp3_frame_end'
        summary['started_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
        summary['source_url'] = {'source1':'https://radio.yunmoan.cn', 'source2':'https://radio.zyghit.cn'}[args.source]
        print('LIVE', json.dumps(summary, ensure_ascii=True), flush=True)
        mic = sc.get_microphone(sc.default_speaker().id, include_loopback=True)
        tui = PtyProcess.spawn(str(ROOT / 'target/debug/examples/latency_probe.exe'), env=env, dimensions=(42,140))
        def drain():
            with (directory / 'terminal.txt').open('w', encoding='utf-8') as output:
                while tui.isalive():
                    try:
                        output.write(tui.read(65536))
                    except (EOFError, OSError):
                        break
        threading.Thread(target=drain, daemon=True).start()
        blocks = []
        timing = []
        # GetBuffer's QPC is the first sample's device timestamp in 100 ns units.
        # Preserve packet gaps instead of treating concatenated audio as continuous.
        from soundcard.mediafoundation import _ffi, _com
        clock_offsets = []
        for _ in range(100):
            a = time.perf_counter()
            wall = time.time()
            b = time.perf_counter()
            clock_offsets.append((b-a, wall-(a+b)/2))
        summary['qpc_to_unix'] = min(clock_offsets)[1]
        with mic.recorder(samplerate=48000, blocksize=4800) as recorder:
            start = time.time()
            while time.time()-start < args.seconds:
                if recorder._capture_available_frames() == 0:
                    time.sleep(.002)
                    continue
                pointer = _ffi.new('BYTE**')
                count = _ffi.new('UINT32*')
                flags = _ffi.new('DWORD*')
                position = _ffi.new('UINT64*')
                qpc = _ffi.new('UINT64*')
                hr = recorder._ppCaptureClient[0][0].lpVtbl.GetBuffer(
                    recorder._ppCaptureClient[0], pointer, count, flags, position, qpc)
                _com.check_error(hr)
                channels = len(set(recorder.channelmap))
                block = np.frombuffer(_ffi.buffer(pointer[0], count[0]*4*channels), dtype='float32').copy().reshape(-1,channels)
                if flags[0] & 2:
                    block[:] = 0
                recorder._capture_release(count[0])
                blocks.append(block[:,0].astype('float32'))
                timing.append([summary['qpc_to_unix']+qpc[0]/1e7, time.time(), len(block), position[0], flags[0]])
        np.save(directory / 'loopback.npy', np.concatenate(blocks))
        np.savetxt(directory / 'capture.csv', timing, delimiter=',')
        (directory / 'metadata.json').write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding='utf-8')
        tui.write('q')
        for _ in range(100):
            if not tui.isalive():
                break
            time.sleep(.1)
        print('CAPTURED', args.name, len(timing), 'blocks', flush=True)
    finally:
        if tui is not None and tui.isalive():
            tui.terminate(force=True)
        try:
            call('/v1/command', 'shutdown')
        except Exception:
            pass
        try:
            host.wait(timeout=5)
        except subprocess.TimeoutExpired:
            host.terminate()
        host_log.close()


def analyze(name):
    import numpy as np
    from scipy import signal
    directory = BASE / name
    meta = json.loads((directory / 'metadata.json').read_text(encoding='utf-8'))
    if 'qpc_to_unix' not in meta:
        raise RuntimeError('This exploratory capture has no device timestamps; do not use for quantitative results.')
    packets = np.loadtxt(directory / 'capture.csv', delimiter=',')
    decoded = np.loadtxt(directory / 'decode.csv', delimiter=',')
    draws = np.loadtxt(directory / 'draw.csv', delimiter=',')
    # The initial exploratory probe recorded MP3 frame ends. The analyzer's
    # 1024-sample hop makes its exact most recent FFT endpoint recoverable.
    if meta.get('sample_position', 'decoded_mp3_frame_end') == 'decoded_mp3_frame_end':
        draws[:,1] -= draws[:,1] % 1024
    rate = int(decoded[0,2])
    if np.any(decoded[:,2] != rate):
        raise RuntimeError('sample rate changed during measurement')
    source = np.fromfile(directory / 'source.f32', dtype='<f4')
    loopback = np.load(directory / 'loopback.npy')
    divisor = math.gcd(8000,rate)
    source8 = signal.resample_poly(source, 8000//divisor, rate//divisor).astype('float64')
    loop8 = signal.resample_poly(loopback, 1, 6).astype('float64')
    source8 -= source8.mean()
    window = 8000
    cumulative = np.r_[0.,np.cumsum(source8**2)]
    source_power = cumulative[window:] - cumulative[:-window]
    starts = np.r_[0,np.cumsum(packets[:-1,2])]
    unique = np.r_[True, np.diff(draws[:,1]) > 0]
    draws = draws[unique & (draws[:,1] > 0)]
    rows = []
    rejected = []
    for start in range(8000*10, len(loop8)-window, 8000*2):
        stop = start + window
        sample_start = start*6
        p = np.searchsorted(starts, sample_start, side='right')-1
        q = np.searchsorted(starts, stop*6, side='right')-1
        stamps = packets[p:q+1,0]
        ideal = packets[p,0] + (starts[p:q+1]-starts[p])/48000
        center_packet = np.searchsorted(starts, sample_start+window*3, side='right')-1
        center_time = packets[center_packet,0]+(sample_start+window*3-starts[center_packet])/48000
        if center_time < meta.get('capture_time_min', float('-inf')) or center_time > meta.get('capture_time_max', float('inf')):
            continue
        if np.max(np.abs(stamps-ideal)) > .001 or np.any(packets[p:q+1,4].astype(int)&4):
            rejected.append({'capture_s':start/8000, 'reason':'packet gap or invalid timestamp'})
            continue
        template = loop8[start:stop].copy()
        template -= template.mean()
        energy = np.sum(template**2)
        if energy < 1e-8:
            rejected.append({'capture_s':start/8000, 'reason':'silent capture'})
            continue
        scores = signal.correlate(source8, template, mode='valid', method='fft') / np.sqrt(np.maximum(source_power*energy,1e-30))
        index = int(np.argmax(scores))
        score = float(scores[index])
        # An independent peak check rules out repeated rhythms being mistaken for the same audio.
        alternative = scores.copy()
        alternative[max(0,index-4000):index+4000] = -1
        second = float(alternative.max())
        if score < .70 or score-second < .15:
            rejected.append({'capture_s':start/8000, 'reason':'ambiguous waveform match', 'correlation':score, 'second':second})
            continue
        source_sample = (index+window/2)*rate/8000
        capture_center = sample_start + window*3
        packet = np.searchsorted(starts,capture_center,side='right')-1
        audio_time = packets[packet,0]+(capture_center-starts[packet])/48000
        if not draws[0,1] <= source_sample <= draws[-1,1]:
            continue
        stage_time = float(np.interp(source_sample,draws[:,1],draws[:,0]))
        rows.append({'capture_s':start/8000+.5, 'source_s':source_sample/rate,
                     'correlation':score, 'second_peak':second, 'stage_minus_audio_ms':(stage_time-audio_time)*1000})
    if not rows:
        raise RuntimeError('No reliable waveform matches: '+json.dumps(rejected[:5]))
    offsets = np.array([row['stage_minus_audio_ms'] for row in rows])
    frame_ms = np.diff(draws[:,0])*1000
    summary = {'run':name, 'matches':len(rows), 'rejected':len(rejected),
        'median_ms':float(np.median(offsets)), 'p05_ms':float(np.percentile(offsets,5)),
        'p95_ms':float(np.percentile(offsets,95)), 'min_ms':float(offsets.min()), 'max_ms':float(offsets.max()),
        'median_waveform_correlation':float(np.median([row['correlation'] for row in rows])),
        'frame_interval_median_ms':float(np.median(frame_ms)), 'frame_interval_p95_ms':float(np.percentile(frame_ms,95)),
        'packet_discontinuities':int(np.count_nonzero(packets[1:,4].astype(int)&1)),
        'captured_seconds':float(len(loopback)/48000), 'source_rate':rate,
        'definition':'stage terminal submission minus WASAPI audio endpoint; positive means stage lags audio',
        'fft_center_correction_ms':1024/rate*1000,
        'note':'Reference is the FFT window end; using its center adds 1024/source_rate seconds. Excludes terminal compositor, monitor scanout and speaker acoustics.'}
    (directory / 'analysis.json').write_text(json.dumps({'summary':summary,'matches':rows,'rejected':rejected},indent=2),encoding='utf-8')
    print(json.dumps(summary,indent=2),flush=True)


def diagnose(name):
    """Decompose saved waveform-aligned measurements without new connections.

    The initial decode term is an extrapolation, not a measured MPV queue size.
    It includes both paths' initial content alignment and buffering.
    """
    import numpy as np
    directory = BASE / name
    analysis = json.loads((directory / 'analysis.json').read_text(encoding='utf-8'))
    decoded = np.loadtxt(directory / 'decode.csv', delimiter=',')
    packets = np.loadtxt(directory / 'capture.csv', delimiter=',')
    rate = decoded[0,2]
    starts = np.r_[0,np.cumsum(packets[:-1,2])]
    # Samples share a content timeline, anchored to actual output via waveform matching.
    first_decode_origin = decoded[0,0]-decoded[0,1]/rate
    output_origins = []
    submission_delays = []
    for row in analysis['matches']:
        captured_sample = row['capture_s']*48000
        packet = np.searchsorted(starts,captured_sample,side='right')-1
        audio_time = packets[packet,0]+(captured_sample-starts[packet])/48000
        output_origins.append(audio_time-row['source_s'])
        decode_time = np.interp(row['source_s']*rate,decoded[:,1],decoded[:,0])
        submission_delays.append((audio_time+row['stage_minus_audio_ms']/1000-decode_time)*1000)
    output_origin = np.median(output_origins)
    slip = (decoded[:,0]-first_decode_origin)-decoded[:,1]/rate
    increments = np.diff(decoded[:,0])-np.diff(decoded[:,1])/rate
    largest = np.argsort(increments)[-5:][::-1]
    initial_ms = (first_decode_origin-output_origin)*1000
    slip_ms = np.median(slip[decoded[:,1] > rate*10])*1000
    submit_ms = np.median(submission_delays)
    rows = analysis['matches']
    drift = np.polyfit([r['capture_s'] for r in rows],
                      [r['stage_minus_audio_ms'] for r in rows],1)[0]*60
    result = {
        'run':name,
        'initial_decode_origin_minus_audio_output_origin_ms':float(initial_ms),
        'steady_analysis_clock_slip_since_first_decode_ms':float(slip_ms),
        'stage_submission_after_decode_median_ms':float(submit_ms),
        'sum_of_component_medians_ms':float(initial_ms+slip_ms+submit_ms),
        'measured_offset_median_ms':analysis['summary']['median_ms'],
        'observed_linear_drift_ms_per_minute':float(drift),
        'waveform_output_origin_range_ms':float(np.ptp(output_origins)*1000),
        'largest_excess_inter_decode_intervals':[
            {'after_pcm_s':float(decoded[i,1]/rate),'extra_wall_ms':float(increments[i]*1000)}
            for i in largest],
        'caveat':'The initial term combines connection alignment and buffering; it is not an isolated libmpv or network delay. Inter-decode waits do not distinguish network, process scheduling, or decoder execution. Component medians need not sum exactly to the measured median.'
    }
    (directory / 'diagnostics.json').write_text(json.dumps(result,indent=2),encoding='utf-8')
    print(json.dumps(result,indent=2),flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build', action='store_true')
    parser.add_argument('--analyze', action='store_true')
    parser.add_argument('--diagnose', action='store_true')
    parser.add_argument('--name', default='run1')
    parser.add_argument('--seconds', type=int, default=65)
    parser.add_argument('--source', choices=['source1','source2'], default='source1')
    args = parser.parse_args()
    if args.build:
        instrument()
    elif args.analyze:
        analyze(args.name)
    elif args.diagnose:
        diagnose(args.name)
    else:
        capture(args)
