use anyhow::{bail, Result};
use std::ffi::c_void;
unsafe extern "C" {
    fn thmr_mp3_create() -> *mut c_void;
    fn thmr_mp3_decode(
        decoder: *mut c_void,
        input: *const u8,
        length: i32,
        pcm: *mut f32,
        info: *mut i32,
    ) -> i32;
    fn thmr_mp3_destroy(decoder: *mut c_void);
}
pub struct Decoder {
    handle: *mut c_void,
    pcm: [f32; 2304],
}
// minimp3 owns all decoder state in this allocation, with no thread-local or shared state.
// Ownership may move between executor threads; decode is only ever called via &mut self.
unsafe impl Send for Decoder {}
impl Decoder {
    pub fn new() -> Result<Self> {
        let handle = unsafe { thmr_mp3_create() };
        if handle.is_null() {
            bail!("could not allocate MP3 decoder");
        }
        Ok(Self {
            handle,
            pcm: [0.0; 2304],
        })
    }
    pub fn decode(&mut self, input: &[u8]) -> (usize, &[f32], u32, usize) {
        let mut info = [0i32; 3];
        let samples = unsafe {
            thmr_mp3_decode(
                self.handle,
                input.as_ptr(),
                input.len().min(i32::MAX as usize) as i32,
                self.pcm.as_mut_ptr(),
                info.as_mut_ptr(),
            )
        };
        let channels = info[2].clamp(0, 2) as usize;
        let length = (samples.max(0) as usize * channels).min(self.pcm.len());
        (
            info[0].max(0) as usize,
            &self.pcm[..length],
            info[1].max(0) as u32,
            channels,
        )
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { thmr_mp3_destroy(self.handle) };
    }
}
