"""Measure the real host and TUI, or the saved pre-replacement binaries.

Windows CPython + numpy/scipy, SoundCard and pywinpty on PYTHONPATH.
Creates isolated settings and measurements under build/latency. Plays audio.
"""
import argparse
import base64
import ctypes
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import threading
import time
import urllib.request

ROOT=Path(__file__).resolve().parents[1]
BASE=ROOT/'build/latency'


class Host:
    def __init__(self,directory,source='source1',baseline=False,audio_output=None,settings=None):
        self.directory=directory
        config=directory/'config';config.mkdir(parents=True,exist_ok=True)
        content=f'source = "{source}"\nvolume = 45.0\n'
        for key,value in (settings or {}).items():content+=f'{key} = {json.dumps(value)}\n'
        (config/'settings.toml').write_text(content,encoding='utf-8')
        self.env=dict(os.environ,THMR_RUNTIME_DIR=str(directory/'runtime'),THMR_CONFIG_DIR=str(config),
            THMR_NO_SYSTEM_MEDIA='1',THMR_AUDIO_TRACE_DIR=str(directory/'host'),THMR_TUI_TRACE_DIR=str(directory),
            THMR_TRACE_DIR=str(directory))
        self.env.pop('THMR_AUDIO_OUTPUT',None)
        if audio_output:self.env['THMR_AUDIO_OUTPUT']=audio_output
        self.env.pop('THMR_TUI_REDUCED_MOTION',None)
        self.executable=BASE/'before-replacement/thmrd.exe' if baseline else ROOT/'target/debug/thmrd.exe'
        self.log=(directory/'host.log').open('wb')
        self.process=subprocess.Popen([str(self.executable)],env=self.env,stdout=self.log,stderr=self.log)
        for _ in range(150):
            if (directory/'runtime/instance.json').exists():break
            if self.process.poll() is not None:raise RuntimeError('host exited during startup; inspect host.log')
            time.sleep(.05)
        self.endpoint=json.loads((directory/'runtime/instance.json').read_text())

    def call(self,command=None,**fields):
        data=None if command is None else json.dumps({'protocolVersion':1,'command':{'command':command,**fields}}).encode()
        path='state' if command is None else 'command'
        request=urllib.request.Request(f'http://127.0.0.1:{self.endpoint["port"]}/v1/{path}',data=data,
            headers={'Authorization':'Bearer '+self.endpoint['token'],'X-THMR-Protocol':'1','Content-Type':'application/json'})
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request,timeout=5) as response:return json.load(response)

    def online(self):
        for _ in range(100):
            state=self.call()
            if state['phase']=='online':return state
            time.sleep(.2)
        raise RuntimeError('host did not become online: '+str(state.get('error')))

    def close(self):
        try:self.call('shutdown')
        except Exception:pass
        try:self.process.wait(timeout=8)
        except subprocess.TimeoutExpired:self.process.terminate();self.process.wait(timeout=3)
        self.log.close()


class AudioSocket:
    """Small native loopback WebSocket reader; bearer values never enter evidence files."""
    def __init__(self,endpoint):
        self.socket=socket.create_connection(('127.0.0.1',endpoint['port']),timeout=3)
        self.socket.settimeout(2)
        key=base64.b64encode(os.urandom(16)).decode()
        request=(f'GET /v1/audio HTTP/1.1\r\nHost: 127.0.0.1:{endpoint["port"]}\r\n'
            f'Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\n'
            f'Authorization: Bearer {endpoint["token"]}\r\nX-THMR-Protocol: 1\r\n\r\n')
        self.socket.sendall(request.encode())
        header=bytearray()
        while not header.endswith(b'\r\n\r\n'):header.extend(self.socket.recv(1))
        if not header.startswith(b'HTTP/1.1 101'):
            self.socket.close();raise RuntimeError('audio WebSocket unavailable')
        expected=base64.b64encode(hashlib.sha1((key+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest())
        assert expected.lower() in bytes(header).lower(),'invalid WebSocket upgrade'

    def read_exact(self,count):
        data=bytearray()
        while len(data)<count:
            block=self.socket.recv(count-len(data))
            if not block:raise EOFError()
            data.extend(block)
        return bytes(data)

    def read(self):
        first,second=self.read_exact(2);opcode=first&15;size=second&127
        if size==126:size=struct.unpack('!H',self.read_exact(2))[0]
        elif size==127:size=struct.unpack('!Q',self.read_exact(8))[0]
        if size>16384:raise RuntimeError('oversized host audio frame')
        mask=self.read_exact(4) if second&128 else None
        payload=self.read_exact(size)
        if mask:payload=bytes(v^mask[i%4] for i,v in enumerate(payload))
        if opcode==8:raise EOFError()
        if opcode!=1:return None
        return json.loads(payload)

    def close(self):
        try:self.socket.shutdown(socket.SHUT_RDWR)
        except OSError:pass
        self.socket.close()


class ProcessStats:
    def __init__(self,pid):
        self.kernel=ctypes.WinDLL('kernel32',use_last_error=True)
        self.psapi=ctypes.WinDLL('psapi',use_last_error=True)
        self.kernel.OpenProcess.argtypes=[ctypes.c_uint32,ctypes.c_int,ctypes.c_uint32]
        self.kernel.OpenProcess.restype=ctypes.c_void_p
        self.kernel.GetProcessTimes.argtypes=[ctypes.c_void_p,*([ctypes.c_void_p]*4)]
        self.kernel.CloseHandle.argtypes=[ctypes.c_void_p]
        self.psapi.GetProcessMemoryInfo.argtypes=[ctypes.c_void_p,ctypes.c_void_p,ctypes.c_uint32]
        self.handle=self.kernel.OpenProcess(0x410,False,pid)

    def sample(self):
        if not self.handle:return None
        times=[ctypes.c_uint64() for _ in range(4)]
        if not self.kernel.GetProcessTimes(self.handle,*[ctypes.byref(v) for v in times]):return None
        class Memory(ctypes.Structure):
            _fields_=[('cb',ctypes.c_uint32),('faults',ctypes.c_uint32)]+[(n,ctypes.c_size_t) for n in
                ['peak_working','working','peak_paged','paged','peak_nonpaged','nonpaged','commit','peak_commit']]
        memory=Memory();memory.cb=ctypes.sizeof(memory)
        if not self.psapi.GetProcessMemoryInfo(self.handle,ctypes.byref(memory),memory.cb):return None
        return {'cpu_seconds':(times[2].value+times[3].value)/1e7,'working_bytes':memory.working,
            'peak_working_bytes':memory.peak_working,'commit_bytes':memory.commit}

    def close(self):
        if self.handle:self.kernel.CloseHandle(self.handle)


def capture(args):
    import numpy as np
    import soundcard as sc
    from soundcard.mediafoundation import _ffi,_com
    from winpty import PtyProcess
    directory=BASE/args.name;directory.mkdir(parents=True,exist_ok=False)
    host=Host(directory,args.source,args.baseline)
    process=None;audio=None;stop=threading.Event();resource_handles=[]
    frames=[];events=[];resources=[];blocks=[];timings=[]
    metadata={'source':args.source,'sample_position':'fft_window_end','capture_rate':48000,
        'baseline':args.baseline,'started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),
        'device':sc.default_speaker().name,'lifecycle':args.lifecycle}
    try:
        host.call('play');host.online()
        if not args.baseline:
            if not list((directory/'host').glob('epoch-*/source.f32')):
                raise RuntimeError('Source PCM trace did not initialize; refusing an unmeasurable capture')
            audio=AudioSocket(host.endpoint)
            def read_audio():
                with (directory/'audio.jsonl').open('w',encoding='utf-8') as output:
                    while not stop.is_set():
                        try:
                            value=audio.read()
                            if value is not None:
                                value['receivedAt']=time.time();frames.append(value)
                                output.write(json.dumps(value)+'\n')
                        except (EOFError,OSError):break
            threading.Thread(target=read_audio,daemon=True).start()
        command=[str(BASE/'before-replacement/latency_probe.exe')] if args.baseline else [str(ROOT/'target/debug/thmr.exe'),'tui']
        process=PtyProcess.spawn(subprocess.list2cmdline(command),env=host.env,dimensions=(42,140))
        def drain():
            with (directory/'terminal.txt').open('w',encoding='utf-8') as output:
                while process.isalive():
                    try:output.write(process.read(65536))
                    except (EOFError,OSError):break
        threading.Thread(target=drain,daemon=True).start()
        resource_handles=[ProcessStats(host.process.pid),ProcessStats(process.pid)]
        offsets=[]
        for _ in range(100):
            a=time.perf_counter();wall=time.time();b=time.perf_counter();offsets.append((b-a,wall-(a+b)/2))
        qpc_offset=min(offsets)[1];metadata['qpc_to_unix']=qpc_offset
        mic=sc.get_microphone(sc.default_speaker().id,include_loopback=True)
        start=time.time();last_resources=0
        def commands():
            plan=[(20,'pause',{}),(22,'play',{}),(45,'set_source',{'source':'source2'}),(70,'pause',{}),(72,'play',{})]
            for at,command,fields in plan:
                if stop.wait(max(0,start+at-time.time())):return
                response=host.call(command,**fields)
                events.append({'wall':time.time(),'command':command,'fields':fields,'phase':response['phase']})
        if args.lifecycle:threading.Thread(target=commands,daemon=True).start()
        print('LIVE',json.dumps(metadata,ensure_ascii=True),flush=True)
        with mic.recorder(samplerate=48000,blocksize=4800) as recorder:
            while time.time()-start<args.seconds:
                if not process.isalive():raise RuntimeError('TUI exited; inspect terminal.txt')
                if time.time()-last_resources>=1:
                    resources.append({'wall':time.time(),'host':resource_handles[0].sample(),'tui':resource_handles[1].sample()})
                    last_resources=time.time()
                if recorder._capture_available_frames()==0:time.sleep(.002);continue
                pointer=_ffi.new('BYTE**');count=_ffi.new('UINT32*');flags=_ffi.new('DWORD*')
                position=_ffi.new('UINT64*');qpc=_ffi.new('UINT64*')
                hr=recorder._ppCaptureClient[0][0].lpVtbl.GetBuffer(recorder._ppCaptureClient[0],pointer,count,flags,position,qpc)
                _com.check_error(hr)
                channels=len(set(recorder.channelmap))
                block=np.frombuffer(_ffi.buffer(pointer[0],count[0]*4*channels),dtype='float32').copy().reshape(-1,channels)
                if flags[0]&2:block[:]=0
                recorder._capture_release(count[0]);blocks.append(block[:,0].astype('float32'))
                timings.append([qpc_offset+qpc[0]/1e7,time.time(),len(block),position[0],flags[0]])
        process.write('q')
        for _ in range(100):
            if not process.isalive():break
            time.sleep(.05)
    finally:
        stop.set()
        if audio:audio.close()
        if process and process.isalive():process.terminate(force=True)
        host.close()
        for handle in resource_handles:handle.close()
    np.save(directory/'loopback.npy',np.concatenate(blocks))
    np.savetxt(directory/'capture.csv',timings,delimiter=',')
    (directory/'metadata.json').write_text(json.dumps(metadata,ensure_ascii=False,indent=2),encoding='utf-8')
    (directory/'events.json').write_text(json.dumps(events,indent=2),encoding='utf-8')
    (directory/'resources.json').write_text(json.dumps(resources,indent=2),encoding='utf-8')
    if not args.baseline:
        assert frames and any(f['active'] for f in frames),'no live host audio frames'
        for frame in frames:
            feature=frame.get('feature')
            assert not feature or feature['endSample']<=frame['playbackSample'],'future feature delivered'
        print('EPOCHS',sorted({f['epoch'] for f in frames}),flush=True)
    print('CAPTURED',args.name,'seconds',sum(row[2] for row in timings)/48000,flush=True)


def analyze(name):
    import numpy as np
    directory=BASE/name;metadata=json.loads((directory/'metadata.json').read_text(encoding='utf-8'))
    spec=importlib.util.spec_from_file_location('legacy',ROOT/'tool/measure_tui_latency.py')
    legacy=importlib.util.module_from_spec(spec);spec.loader.exec_module(legacy)
    results=[]
    if metadata['baseline']:
        legacy.analyze(name);results.append(json.loads((directory/'analysis.json').read_text()))
    else:
        draws=np.loadtxt(directory/'tui-draw.csv',delimiter=',')
        for epoch in sorted({int(v) for v in draws[:,1] if v>0}):
            source=directory/'host'/f'epoch-{epoch}'
            selected=draws[(draws[:,1]==epoch)&(draws[:,2]>0)]
            if len(selected)<20:continue
            child=directory/f'epoch-{epoch}';child.mkdir(exist_ok=True)
            if not all((source/filename).exists() for filename in ['source.f32','decode.csv']):
                print('MISSING_SOURCE_EVIDENCE',epoch,flush=True)
                continue
            for filename in ['source.f32','decode.csv']:shutil.copyfile(source/filename,child/filename)
            for filename in ['loopback.npy','capture.csv']:shutil.copyfile(directory/filename,child/filename)
            np.savetxt(child/'draw.csv',np.column_stack([selected[:,0],selected[:,2],np.zeros(len(selected))]),delimiter=',')
            child_meta=dict(metadata,capture_time_min=float(selected[0,0]),capture_time_max=float(selected[-1,0]))
            (child/'metadata.json').write_text(json.dumps(child_meta),encoding='utf-8')
            try:
                legacy.analyze(str(child.relative_to(BASE)))
            except RuntimeError as error:
                print('INSUFFICIENT',epoch,str(error)[:200]);continue
            result=json.loads((child/'analysis.json').read_text());result['epoch']=epoch
            packets=np.loadtxt(directory/'capture.csv',delimiter=',')
            starts=np.r_[0,np.cumsum(packets[:-1,2])]
            decoded=np.loadtxt(source/'decode.csv',delimiter=',')
            presentation=np.loadtxt(source/'present.csv',delimiter=',')
            active=presentation[(presentation[:,4]==1)&(presentation[:,2]>0)]
            active=active[np.r_[True,np.diff(active[:,2])>0]]
            selection_offsets=[];preparation=[]
            for row in result['matches']:
                index=row['capture_s']*48000;packet=np.searchsorted(starts,index,side='right')-1
                output_time=packets[packet,0]+(index-starts[packet])/48000
                sample=row['source_s']*decoded[0,2]
                selection_time=np.interp(sample,active[:,2],active[:,0])
                computed=np.interp(sample,decoded[:,1],decoded[:,0])
                selection_offsets.append((selection_time-output_time)*1000)
                preparation.append((output_time-computed)*1000)
                row['host_selection_minus_audio_ms']=float(selection_offsets[-1])
            messages=[]
            with (directory/'audio.jsonl').open(encoding='utf-8') as stream:
                for line in stream:
                    frame=json.loads(line)
                    if frame['epoch']==epoch:messages.append(frame)
            requests=sum(',request,' in line for line in (source/'network.csv').read_text().splitlines())
            assert requests==1,'multiple upstream requests in one playback epoch'
            ahead=max((f['queue']['decodedSamples']-f['playbackSample'])/f['sampleRate'] for f in messages if f['sampleRate']>0)
            assert ahead<3.1,'unbounded analysis lookahead'
            result['host']={'selection_median_ms':float(np.median(selection_offsets)),
                'selection_p95_ms':float(np.percentile(selection_offsets,95)),
                'prepared_before_output_median_ms':float(np.median(preparation)),
                'prepared_before_output_min_ms':float(np.min(preparation)),
                'upstream_requests':requests,'analysis_ahead_peak_s':float(ahead),
                'retired_features':max(f['queue']['retiredFeatures'] for f in messages),
                'max_pcm_bytes':max(f['queue']['maxPcmBytes'] for f in messages),
                'max_pending_features':max(f['queue']['maxPendingFeatures'] for f in messages)}
            (child/'analysis.json').write_text(json.dumps(result,indent=2),encoding='utf-8')
            results.append(result)
            print('HOST_TIMING',json.dumps(result['host'],indent=2),flush=True)
        (directory/'analysis.json').write_text(json.dumps({'epochs':results},indent=2),encoding='utf-8')
    resources=json.loads((directory/'resources.json').read_text())
    summary={}
    for role in ['host','tui']:
        valid=[row for row in resources if row[role] is not None]
        if len(valid)>1:
            summary[role]={'cpu_percent_one_core':(valid[-1][role]['cpu_seconds']-valid[0][role]['cpu_seconds'])/(valid[-1]['wall']-valid[0]['wall'])*100,
                'working_mib_median':float(np.median([row[role]['working_bytes']/1048576 for row in valid])),
                'working_mib_peak':max(row[role]['peak_working_bytes']/1048576 for row in valid)}
    (directory/'resource-summary.json').write_text(json.dumps(summary,indent=2),encoding='utf-8')
    print('RESOURCES',json.dumps(summary,indent=2),flush=True)
    assert results,'no usable timing epochs'


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--name',required=True)
    parser.add_argument('--seconds',type=int,default=75)
    parser.add_argument('--source',choices=['source1','source2'],default='source1')
    parser.add_argument('--baseline',action='store_true')
    parser.add_argument('--lifecycle',action='store_true')
    parser.add_argument('--analyze',action='store_true')
    args=parser.parse_args()
    if args.analyze:analyze(args.name)
    else:capture(args)
