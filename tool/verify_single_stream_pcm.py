"""EXPERIMENT ONLY. Verify one upstream, shared decoded PCM and playback-clock features.

Windows: use the same CPython/PYTHONPATH as measure_tui_latency.py.
Run --build, then capture with a fresh --name, then --analyze.
Normal host, TUI sources and user settings are not changed.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / 'build/latency'


def build():
    if 'struct Decoder {' not in (ROOT / 'crates/thmr-cli/src/tui/spectrum.rs').read_text(encoding='utf-8'):
        executable = ROOT / 'target/debug/examples/single_stream_pcm.exe'
        if not executable.exists():
            raise RuntimeError('This historical prototype predates host integration. Use tool/measure_host_audio.py to verify the production implementation.')
        print('Reusing historical prototype executable; production verification: tool/measure_host_audio.py', flush=True)
        return
    dest = BASE / 'single-stream-source'
    dest.mkdir(parents=True, exist_ok=True)
    names = ['battlefield','choreography','danmaku','model','render','spectrum','spectrum_view']
    declarations = []
    for name in names:
        source = (ROOT / f'crates/thmr-cli/src/tui/{name}.rs').read_text(encoding='utf-8')
        (dest / f'{name}.rs').write_text(source, encoding='utf-8')
        declarations.append(f'#[path="../../../build/latency/single-stream-source/{name}.rs"] mod {name};')
    with (dest / 'spectrum.rs').open('a', encoding='utf-8') as stream:
        stream.write('''
// Experiment access wrapper. The production Decoder and Analyzer are unchanged.
pub struct AheadProcessor { decoder: Decoder, analyzer: Analyzer, end: u64 }
impl AheadProcessor {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {decoder: Decoder::new()?, analyzer: Analyzer::default(), end: 0})
    }
    pub fn decode(&mut self, input: &[u8]) -> anyhow::Result<(usize,Vec<f32>,u32,usize,Option<(u64,DisplaySpectrum,ChannelLevels,MusicalFeatures)>)> {
        let (advance,samples,rate,channels)=self.decoder.decode(input);
        let pcm=samples.to_vec();
        let mut prepared=None;
        if !pcm.is_empty() {
            self.end+=(pcm.len()/channels) as u64;
            if self.analyzer.add(&pcm,rate,channels).is_some() {
                prepared=Some((self.end-self.analyzer.hop as u64,self.analyzer.display.frame,
                    self.analyzer.levels,self.analyzer.music.features));
            }
        }
        Ok((advance,pcm,rate,channels,prepared))
    }
}
''')
    example = ROOT / 'crates/thmr-cli/examples/single_stream_pcm.rs'
    example.parent.mkdir(exist_ok=True)
    source = (ROOT / 'tool/prototypes/single_stream_pcm.rs').read_text(encoding='utf-8')
    example.write_text(source.replace('// MODULES','\n'.join(declarations)), encoding='utf-8')
    try:
        subprocess.run(['cargo','build','-p','thmr-cli','--example','single_stream_pcm'], cwd=ROOT, check=True)
    finally:
        example.unlink(missing_ok=True)


def capture(args):
    import numpy as np
    import soundcard as sc
    from soundcard.mediafoundation import _ffi, _com
    from winpty import PtyProcess
    directory = BASE / args.name
    directory.mkdir(parents=True, exist_ok=False)
    origin = {'source1':'https://radio.yunmoan.cn','source2':'https://radio.zyghit.cn'}[args.source]
    env = dict(os.environ, THMR_TRACE_DIR=str(directory),
        THMR_PROBE_SOURCE=origin+'/radio/touhou/128k', THMR_PROBE_SECONDS=str(args.seconds),
        THMR_MPV_LIBRARY=str(ROOT/'target/debug/libmpv-2.dll'))
    if args.stress:
        env['THMR_PROBE_STRESS']='1'
    else:
        env.pop('THMR_PROBE_STRESS',None)
    offsets = []
    for _ in range(100):
        before=time.perf_counter(); wall=time.time(); after=time.perf_counter()
        offsets.append((after-before,wall-(before+after)/2))
    qpc_offset = min(offsets)[1]
    metadata = {'source':args.source,'source_url':origin,'capture_rate':48000,
        'qpc_to_unix':qpc_offset,'sample_position':'fft_window_end','prototype':'single_stream_pcm',
        'device':sc.default_speaker().name,'started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),
        'stress':args.stress}
    print('CAPTURING',json.dumps(metadata,ensure_ascii=True),flush=True)
    mic = sc.get_microphone(sc.default_speaker().id,include_loopback=True)
    blocks=[]; timings=[]
    process=None
    try:
        with mic.recorder(samplerate=48000,blocksize=4800) as recorder:
            process=PtyProcess.spawn(str(ROOT/'target/debug/examples/single_stream_pcm.exe'),
                env=env,dimensions=(42,140))
            def drain():
                with (directory/'terminal.txt').open('w',encoding='utf-8') as output:
                    while process.isalive():
                        try:output.write(process.read(65536))
                        except (EOFError,OSError):break
            threading.Thread(target=drain,daemon=True).start()
            start=time.time()
            while time.time()-start<args.seconds+12:
                if not process.isalive():break
                if recorder._capture_available_frames()==0:
                    time.sleep(.002);continue
                pointer=_ffi.new('BYTE**');count=_ffi.new('UINT32*');flags=_ffi.new('DWORD*')
                position=_ffi.new('UINT64*');qpc=_ffi.new('UINT64*')
                hr=recorder._ppCaptureClient[0][0].lpVtbl.GetBuffer(
                    recorder._ppCaptureClient[0],pointer,count,flags,position,qpc)
                _com.check_error(hr)
                channels=len(set(recorder.channelmap))
                block=np.frombuffer(_ffi.buffer(pointer[0],count[0]*4*channels),dtype='float32').copy().reshape(-1,channels)
                if flags[0]&2:block[:]=0
                recorder._capture_release(count[0])
                blocks.append(block[:,0].astype('float32'))
                timings.append([qpc_offset+qpc[0]/1e7,time.time(),len(block),position[0],flags[0]])
    finally:
        if process is not None and process.isalive():process.terminate(force=True)
    if blocks:
        np.save(directory/'loopback.npy',np.concatenate(blocks))
        np.savetxt(directory/'capture.csv',timings,delimiter=',')
    (directory/'metadata.json').write_text(json.dumps(metadata,ensure_ascii=False,indent=2),encoding='utf-8')
    if not (directory/'prototype.json').exists():
        tail=(directory/'terminal.txt').read_text(encoding='utf-8')[-2500:]
        raise RuntimeError('Prototype did not finish; terminal tail: '+tail)
    print('FINISHED',json.loads((directory/'prototype.json').read_text()),flush=True)


def analyze(name):
    import numpy as np
    spec=importlib.util.spec_from_file_location('latency',ROOT/'tool/measure_tui_latency.py')
    latency=importlib.util.module_from_spec(spec);spec.loader.exec_module(latency)
    latency.analyze(name)
    directory=BASE/name
    metrics=np.loadtxt(directory/'metrics.csv',delimiter=',')
    result=json.loads((directory/'analysis.json').read_text())
    prototype=json.loads((directory/'prototype.json').read_text())
    rows=result['matches']
    # Calculate advance preparation against captured audio's hardware timeline.
    packets=np.loadtxt(directory/'capture.csv',delimiter=',')
    decoded=np.loadtxt(directory/'decode.csv',delimiter=',')
    starts=np.r_[0,np.cumsum(packets[:-1,2])]
    leads=[]
    scheduled_offsets=[]
    active_rows=metrics[(metrics[:,6]==0)&(metrics[:,7]==0)&(metrics[:,2]>0)]
    unique=np.r_[True,np.diff(active_rows[:,2])>0]
    active_rows=active_rows[unique]
    for row in rows:
        sample=row['capture_s']*48000
        packet=np.searchsorted(starts,sample,side='right')-1
        output_time=packets[packet,0]+(sample-starts[packet])/48000
        computed=np.interp(row['source_s']*decoded[0,2],decoded[:,1],decoded[:,0])
        leads.append((output_time-computed)*1000)
        selected_time=np.interp(row['source_s'],active_rows[:,2],active_rows[:,0])
        scheduled_offsets.append((selected_time-output_time)*1000)
        row['scheduled_feature_minus_audio_ms']=float(scheduled_offsets[-1])
        row['prepared_before_audio_output_ms']=float(leads[-1])
        row['audio_output_unix']=float(output_time)
    active=(metrics[:,6]==0)&(metrics[:,7]==0)&(metrics[:,1]>1)&(metrics[:,2]>0)
    clock_errors=(metrics[active,1]-metrics[active,2])*1000
    network=(directory/'network.csv').read_text().splitlines()
    requests=sum(',request,' in line for line in network)
    result['prototype_validation']={
        'logged_upstream_requests':requests,
        'prepared_before_audio_output_median_ms':float(np.median(leads)),
        'prepared_before_audio_output_min_ms':float(np.min(leads)),
        'scheduled_feature_minus_audio_median_ms':float(np.median(scheduled_offsets)),
        'scheduled_feature_minus_audio_p05_ms':float(np.percentile(scheduled_offsets,5)),
        'scheduled_feature_minus_audio_p95_ms':float(np.percentile(scheduled_offsets,95)),
        'feature_age_vs_player_clock_median_ms':float(np.median(clock_errors)),
        'feature_age_vs_player_clock_p95_ms':float(np.percentile(clock_errors,95)),
        'feature_age_vs_player_clock_min_ms':float(np.min(clock_errors)),
        'prototype':prototype,
    }
    assert requests==1,requests
    assert prototype['max_analysis_ahead_s']<3.1,prototype
    assert prototype['retired_features']>100,prototype
    assert np.median(leads)>0,leads
    assert np.min(clock_errors)>-1,'selected a future feature'
    assert np.all(np.diff(metrics[:,9])>=0),'retirement counter moved backwards'
    if prototype['stress']:
        paused=metrics[metrics[:,6]==1]
        assert len(paused)>5,'pause scenario did not execute'
        assert np.ptp(paused[:,2])<1e-6,'feature cursor advanced during explicit pause'
        assert np.ptp(paused[:,9])==0,'features retired during explicit pause'
        assert any(',injected_ingest_stall_start,' in line for line in network)
        assert any(',injected_ingest_stall_end,' in line for line in network)
        stalled_at=float(next(line.split(',')[0] for line in network if ',injected_ingest_stall_start,' in line))
        buffering_after_stall=metrics[(metrics[:,0]>stalled_at)&(metrics[:,7]==1)]
        assert len(buffering_after_stall)>0,'injected stall did not exhaust playback buffer'
        result['prototype_validation']['explicit_pause_cursor_delta_s']=float(np.ptp(paused[:,2]))
        result['prototype_validation']['buffering_frames_after_injected_stall']=int(len(buffering_after_stall))
        before=[r['scheduled_feature_minus_audio_ms'] for r in rows if r['audio_output_unix']<stalled_at-2]
        after=[r['scheduled_feature_minus_audio_ms'] for r in rows if r['audio_output_unix']>stalled_at+11]
        assert len(before)>=3 and len(after)>=3,'insufficient before/after waveform matches'
        recovery_shift=float(np.median(after)-np.median(before))
        assert abs(recovery_shift)<75,'persistent offset introduced by injected stall'
        result['prototype_validation']['before_stall_scheduled_median_ms']=float(np.median(before))
        result['prototype_validation']['after_recovery_scheduled_median_ms']=float(np.median(after))
        result['prototype_validation']['recovery_offset_change_ms']=recovery_shift
    (directory/'analysis.json').write_text(json.dumps(result,indent=2),encoding='utf-8')
    print('VALIDATION',json.dumps(result['prototype_validation'],indent=2),flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build',action='store_true')
    parser.add_argument('--analyze',action='store_true')
    parser.add_argument('--name',default='pcm1')
    parser.add_argument('--source',choices=['source1','source2'],default='source1')
    parser.add_argument('--seconds',type=int,default=75)
    parser.add_argument('--stress',action='store_true')
    args=parser.parse_args()
    if args.build:build()
    elif args.analyze:analyze(args.name)
    else:capture(args)
