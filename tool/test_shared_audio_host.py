"""Real thmrd/libmpv regression: one stream per epoch, pause, format/source switch and reconnect.

Uses local MP3 fixtures and libmpv's timed null output, no external service or sound.
"""
from collections import Counter
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
import json
from pathlib import Path
import tempfile
import threading
import time
from measure_host_audio import Host,AudioSocket,ROOT


class Radio:
    def __init__(self,fixture,bytes_per_second):
        self.data=(ROOT/'test/fixtures'/fixture).read_bytes()
        self.requests=Counter();self.active=0;self.generation=0;self.lock=threading.Lock()
        radio=self
        class Handler(BaseHTTPRequestHandler):
            protocol_version='HTTP/1.1'
            def log_message(self,*args):pass
            def do_GET(self):
                self.close_connection=True
                if self.path.startswith('/api/'):
                    body=json.dumps({'station':{'id':'touhou','bitrates':[{'id':'128k','enabled':True},{'id':'64k','enabled':True}]},
                        'current':{'title':'fixture'},'queue':[]}).encode()
                    self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
                with radio.lock:
                    radio.requests[self.path]+=1;radio.active+=1;generation=radio.generation
                try:
                    self.send_response(200);self.send_header('Content-Type','audio/mpeg');self.send_header('Transfer-Encoding','chunked');self.end_headers()
                    while True:
                        for offset in range(0,len(radio.data),1024):
                            if radio.generation!=generation:self.close_connection=True;return
                            chunk=radio.data[offset:offset+1024]
                            self.wfile.write(f'{len(chunk):x}\r\n'.encode()+chunk+b'\r\n');self.wfile.flush()
                            time.sleep(len(chunk)/bytes_per_second)
                except (ConnectionError,OSError):pass
                finally:
                    with radio.lock:radio.active-=1
        self.server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
        self.server.daemon_threads=True
        threading.Thread(target=self.server.serve_forever,daemon=True).start()
        self.origin=f'http://127.0.0.1:{self.server.server_port}'

    def interrupt(self):self.generation+=1
    def close(self):self.interrupt();self.server.shutdown();self.server.server_close()


def until(predicate,timeout=12):
    deadline=time.monotonic()+timeout
    while time.monotonic()<deadline:
        if predicate():return
        time.sleep(.03)
    raise AssertionError('condition did not become true')


def main():
    first=Radio('sine_1000hz_44100_stereo.mp3',16000)
    second=Radio('sine_440hz_48000_mono.mp3',12000)
    host=None;audio=None
    try:
        with tempfile.TemporaryDirectory(prefix='thmr-audio-regression-',dir=ROOT/'build') as temporary:
            host=Host(Path(temporary),audio_output='null',settings={'source1':first.origin,'source2':second.origin})
            host.call('play');host.online();audio=AudioSocket(host.endpoint)
            def frame_matching(predicate,timeout=15):
                deadline=time.monotonic()+timeout
                while time.monotonic()<deadline:
                    frame=audio.read()
                    if frame is None:continue
                    if frame.get('feature'):
                        assert frame['feature']['endSample']<=frame['playbackSample']
                        assert len(frame['feature']['display']['bands'])==48
                    if predicate(frame):return frame
                raise AssertionError('audio frame predicate timed out')
            initial=frame_matching(lambda f:f['active']);epoch=initial['epoch']
            assert initial['sampleRate']==44100
            assert sum(first.requests.values())==1
            for _ in range(8):
                frame=audio.read();assert frame['epoch']==epoch
            print('PASS: real decoder, shared PCM, timed libmpv and authenticated audio stream')

            host.call('pause');paused=frame_matching(lambda f:f['epoch']>epoch and not f['active'])
            epoch=paused['epoch'];assert paused['feature'] is None and paused['queue']['pendingFeatures']==0
            until(lambda:first.active==0)
            host.call('set_source',source='source2');assert sum(second.requests.values())==0
            assert host.call()['phase']=='idle'
            host.call('play');host.online();changed=frame_matching(lambda f:f['epoch']>epoch and f['active'])
            assert changed['sampleRate']==48000
            assert changed['feature']['levels']['rms'][0]==changed['feature']['levels']['rms'][1]
            assert sum(second.requests.values())==1
            epoch=changed['epoch'];print('PASS: pause cancels download; resume/source change resets epoch and renegotiates mono 48 kHz')

            until(lambda:bool((host.call().get('stationStatus') or {}).get('station')))
            host.call('set_profile',profileId='64k')
            quality=frame_matching(lambda f:f['epoch']>epoch and f['active'])
            epoch=quality['epoch'];assert second.requests['/radio/touhou/64k']==1
            print('PASS: bitrate change uses a new content epoch and a single new upstream')

            second.interrupt()
            recovered=frame_matching(lambda f:f['epoch']>epoch and f['active'],timeout=18)
            assert recovered['epoch']>epoch
            assert second.requests['/radio/touhou/64k']==2
            assert recovered['queue']['decodedSamples']<48000*4
            print('PASS: interrupted upstream reconnects, discards old metrics and restarts the sample clock')
            host.call('stop');stopped=frame_matching(lambda f:not f['active'] and f['epoch']>recovered['epoch'])
            until(lambda:second.active==0)
            assert stopped['feature'] is None
            assert host.call()['phase']=='idle'
            audio.close();audio=None;host.close();host=None
            traces=list((Path(temporary)/'host').glob('epoch-*'))
            assert traces and all((trace/'source.f32').exists() and (trace/'decode.csv').exists() for trace in traces)
            print('PASS: stop and shutdown release streams, endpoint and playback')
    finally:
        if audio:audio.close()
        if host:host.close()
        first.close();second.close()


if __name__=='__main__':main()
